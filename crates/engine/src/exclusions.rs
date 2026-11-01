//! Exclusions: items of an account that are neither downloaded, cached nor
//! uploaded.
//!
//! Two layers apply them:
//!
//! - rclone filters on the hidden mount (`_filter` of `mount/mount`): the
//!   excluded remote items are not listed nor downloaded. Changing a rule
//!   remounts the hidden mount only.
//! - `nuage-fs` at the user's folder: an excluded item created or renamed
//!   locally is kept in `data/local-only/<account>` and never reaches rclone
//!   (rclone filters alone would upload it).
//!
//! Rules are saved per account in `config.toml`, with the default patterns
//! ([`nuage_core::DEFAULT_PATTERNS`]) unless turned off.

use nuage_core::{
    AccountId, DEFAULT_PATTERNS, Error, Exclusion, ExclusionRule, ExclusionSet, Result, SyncState,
};

use crate::config::{StoredExclusion, StoredRule};
use crate::{Inner, State};

/// The rule as saved, after checking it.
fn stored(rule: &ExclusionRule) -> Result<StoredRule> {
    match rule {
        ExclusionRule::Path(p) => {
            let path: Vec<&str> = p
                .path
                .split('/')
                .filter(|c| !c.is_empty() && *c != ".")
                .collect();
            if path.is_empty() {
                return Err(Error::InvalidArgument(
                    "The whole account cannot be excluded; unmount it instead (nuage unmount)."
                        .to_owned(),
                ));
            }
            if path.contains(&"..") {
                return Err(Error::InvalidArgument(format!(
                    "\"{}\" is not a path inside the account.",
                    p.path
                )));
            }
            Ok(StoredRule::Path(path.join("/")))
        }
        ExclusionRule::Glob(g) => {
            let g = g.trim();
            if g.is_empty() || g.trim_matches('/').is_empty() || g == "*" || g == "**" {
                return Err(Error::InvalidArgument(format!(
                    "\"{g}\" would exclude everything. Unmount the account instead (nuage unmount)."
                )));
            }
            Ok(StoredRule::Glob(g.to_owned()))
        }
    }
}

impl Inner {
    pub(crate) async fn add_exclusion(
        &self,
        account: &AccountId,
        rule: ExclusionRule,
        force: bool,
    ) -> Result<()> {
        let rule = stored(&rule)?;
        self.change_exclusions(account, force, |record| {
            if record.exclusions.iter().any(|e| e.rule == rule) {
                return Err(Error::AlreadyExists(
                    "This exclusion exists already.".to_owned(),
                ));
            }
            if let StoredRule::Glob(g) = &rule
                && record.default_exclusions
                && DEFAULT_PATTERNS.contains(&g.as_str())
            {
                return Err(Error::AlreadyExists(format!(
                    "\"{g}\" is one of the default exclusions, already applied."
                )));
            }
            record.exclusions.push(StoredExclusion {
                rule: rule.clone(),
                created_at: chrono::Utc::now(),
            });
            Ok(())
        })
        .await
    }

    pub(crate) async fn remove_exclusion(
        &self,
        account: &AccountId,
        rule: ExclusionRule,
        force: bool,
    ) -> Result<()> {
        let rule = stored(&rule)?;
        self.change_exclusions(account, force, |record| {
            let before = record.exclusions.len();
            record.exclusions.retain(|e| e.rule != rule);
            if record.exclusions.len() == before {
                if let StoredRule::Glob(g) = &rule
                    && DEFAULT_PATTERNS.contains(&g.as_str())
                {
                    return Err(Error::InvalidArgument(format!(
                        "\"{g}\" is a default exclusion. Turn the defaults off with: nuage config set \"{}:default-exclusions\" off",
                        record.account.display_name
                    )));
                }
                return Err(Error::NotFound("No such exclusion.".to_owned()));
            }
            Ok(())
        })
        .await
    }

    /// Turns the default patterns on or off.
    pub(crate) async fn set_default_exclusions(
        &self,
        account: &AccountId,
        on: bool,
        force: bool,
    ) -> Result<()> {
        self.change_exclusions(account, force, |record| {
            record.default_exclusions = on;
            Ok(())
        })
        .await
    }

    /// Applies `change` to the rules of `account`, saves them, gives them to
    /// `nuage-fs` and remounts the hidden rclone mount.
    async fn change_exclusions(
        &self,
        account: &AccountId,
        force: bool,
        change: impl FnOnce(&mut crate::config::AccountRecord) -> Result<()>,
    ) -> Result<()> {
        let id = {
            let st = self.state.lock().await;
            st.record(st.resolve(account)?).account.id.clone()
        };
        if !force {
            let pending = self.pending_of(&id).await;
            if pending > 0 {
                return Err(Error::Busy(format!(
                    "{pending} file(s) are waiting to be uploaded. Applying the exclusion remounts the account; wait for them, or confirm with --yes (they upload after the remount)."
                )));
            }
        }
        let mut st = self.state.lock().await;
        let i = st.resolve(&id)?;
        let mut record = st.config.accounts[i].clone();
        change(&mut record)?;
        st.config.accounts[i] = record;
        self.save(&st)?;
        self.apply_exclusions(&mut st, i).await
    }

    /// Gives the rules to `nuage-fs` at once, then remounts the hidden mount
    /// with the matching rclone filters.
    async fn apply_exclusions(&self, st: &mut State, i: usize) -> Result<()> {
        let set = ExclusionSet::new(&st.record(i).exclusion_rules());
        let id = st.record(i).account.id.clone();
        if let Some(fs) = st.runtime.get(&id).and_then(|r| r.fs.as_ref()) {
            // Blocking: moves no longer excluded local files to rclone.
            tokio::task::block_in_place(|| fs.set_exclusions(set));
        }
        self.remount_hidden(st, i).await
    }

    /// Uploads pending (uploading, queued or held by a pause) for `id`.
    async fn pending_of(&self, id: &AccountId) -> u32 {
        let report = self.poll_status().await;
        report
            .accounts
            .iter()
            .find(|a| &a.account.id == id)
            .map_or(0, |a| match a.state {
                SyncState::Syncing { uploads, queued } => uploads + queued,
                SyncState::Paused { queued } => queued,
                _ => 0,
            })
    }

    pub(crate) async fn list_exclusions(
        &self,
        account: Option<&AccountId>,
    ) -> Result<Vec<Exclusion>> {
        let st = self.state.lock().await;
        let indices: Vec<usize> = match account {
            Some(id) => vec![st.resolve(id)?],
            None => (0..st.config.accounts.len()).collect(),
        };
        let mut out = Vec::new();
        for i in indices {
            let record = st.record(i);
            let id = &record.account.id;
            if record.default_exclusions {
                out.extend(DEFAULT_PATTERNS.iter().map(|p| Exclusion {
                    account: id.clone(),
                    rule: ExclusionRule::Glob((*p).to_owned()),
                    created_at: chrono::DateTime::UNIX_EPOCH,
                    default: true,
                }));
            }
            out.extend(record.exclusions.iter().map(|e| Exclusion {
                account: id.clone(),
                rule: match &e.rule {
                    StoredRule::Path(p) => {
                        ExclusionRule::Path(nuage_core::RemotePath::new(id.clone(), p.clone()))
                    }
                    StoredRule::Glob(g) => ExclusionRule::Glob(g.clone()),
                },
                created_at: e.created_at,
                default: false,
            }));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuage_core::RemotePath;

    #[test]
    fn rules_are_checked() {
        let path = |p: &str| ExclusionRule::Path(RemotePath::new(AccountId::new("a"), p));
        assert_eq!(
            stored(&path("/Archives/old/")).unwrap(),
            StoredRule::Path("Archives/old".into())
        );
        assert!(stored(&path("/")).is_err());
        assert!(stored(&path("a/../b")).is_err());
        assert!(stored(&ExclusionRule::Glob("*".into())).is_err());
        assert!(stored(&ExclusionRule::Glob(" / ".into())).is_err());
        assert_eq!(
            stored(&ExclusionRule::Glob(" node_modules/ ".into())).unwrap(),
            StoredRule::Glob("node_modules/".into())
        );
    }
}
