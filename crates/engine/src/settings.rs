//! `nuage config get/set`.
//!
//! | Key | Value |
//! |---|---|
//! | `cache-limit` | cache limit of new accounts (`20G`) |
//! | `<account>:cache-limit` | cache limit of one account |
//! | `cache-max-age` | maximum age of cached files (`30d`) |
//! | `<account>:cache-max-age` | the same for one account (`default`: the global one) |
//! | `<account>:default-exclusions` | `on` or `off` |
//! | `quit-with-pending` | `finish-in-background`, `ask` or `quit-now` |
//!
//! Changing the cache settings of a mounted account remounts its hidden
//! rclone mount (the folder stays; pending uploads go on after it).

use nuage_core::{AccountId, ConfigEntry, Error, Result};

use crate::config::QuitBehavior;
use crate::units::{format_duration, format_size, parse_duration, parse_size};
use crate::{Inner, State};

const CACHE_LIMIT: &str = "cache-limit";
const CACHE_MAX_AGE: &str = "cache-max-age";
const QUIT_WITH_PENDING: &str = "quit-with-pending";
const DEFAULT_EXCLUSIONS: &str = "default-exclusions";

fn entry(key: impl Into<String>, value: impl Into<String>) -> ConfigEntry {
    ConfigEntry {
        key: key.into(),
        value: value.into(),
    }
}

fn all_entries(st: &State, recommended_age: Option<u64>) -> Vec<ConfigEntry> {
    let s = &st.config.settings;
    let mut entries = vec![
        entry(CACHE_LIMIT, format_size(s.default_cache_limit)),
        entry(
            CACHE_MAX_AGE,
            s.cache_max_age_secs
                .or(recommended_age)
                .map_or_else(|| "default".to_owned(), format_duration),
        ),
        entry(QUIT_WITH_PENDING, s.quit_with_pending.as_str()),
    ];
    for r in &st.config.accounts {
        entries.push(entry(
            format!("{}:{CACHE_LIMIT}", r.account.display_name),
            format_size(r.account.cache_limit),
        ));
        entries.push(entry(
            format!("{}:{CACHE_MAX_AGE}", r.account.display_name),
            age_text(r.cache_max_age_secs),
        ));
        entries.push(entry(
            format!("{}:{DEFAULT_EXCLUSIONS}", r.account.display_name),
            on_off(r.default_exclusions),
        ));
    }
    entries
}

fn age_text(secs: Option<u64>) -> String {
    secs.map_or_else(|| "default".to_owned(), format_duration)
}

/// `default` or a duration.
fn parse_age(value: &str) -> Result<Option<u64>> {
    if value == "default" {
        return Ok(None);
    }
    parse_duration(value).map(Some).ok_or_else(|| {
        Error::InvalidArgument(format!(
            "\"{value}\" is not a duration. Write it like 30d, 12h or 2w, or `default`."
        ))
    })
}

fn unknown_key(key: &str) -> Error {
    Error::InvalidArgument(format!(
        "Unknown setting \"{key}\". Settings: {CACHE_LIMIT}, <account>:{CACHE_LIMIT}, {CACHE_MAX_AGE}, <account>:{CACHE_MAX_AGE}, <account>:{DEFAULT_EXCLUSIONS}, {QUIT_WITH_PENDING}."
    ))
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

fn bad_size(value: &str) -> Error {
    Error::InvalidArgument(format!(
        "\"{value}\" is not a size. Write it like 20G, 500M or 1T."
    ))
}

impl Inner {
    fn recommended_cache_age(&self) -> Option<u64> {
        nuage_core::VfsOptions::default().cache_max_age_secs
    }

    pub(crate) async fn config_get(&self, key: Option<&str>) -> Result<Vec<ConfigEntry>> {
        let st = self.state.lock().await;
        let entries = all_entries(&st, self.recommended_cache_age());
        let Some(key) = key else {
            return Ok(entries);
        };
        if let Some((account, setting)) = key.rsplit_once(':') {
            let i = st.resolve(&AccountId::new(account))?;
            let r = st.record(i);
            let name = &r.account.display_name;
            return match setting {
                CACHE_LIMIT => Ok(vec![entry(
                    format!("{name}:{CACHE_LIMIT}"),
                    format_size(r.account.cache_limit),
                )]),
                CACHE_MAX_AGE => Ok(vec![entry(
                    format!("{name}:{CACHE_MAX_AGE}"),
                    age_text(r.cache_max_age_secs),
                )]),
                DEFAULT_EXCLUSIONS => Ok(vec![entry(
                    format!("{name}:{DEFAULT_EXCLUSIONS}"),
                    on_off(r.default_exclusions),
                )]),
                _ => Err(unknown_key(key)),
            };
        }
        entries
            .into_iter()
            .find(|e| e.key == key)
            .map(|e| vec![e])
            .ok_or_else(|| unknown_key(key))
    }

    pub(crate) async fn config_set(&self, key: &str, value: &str) -> Result<Vec<ConfigEntry>> {
        if let Some((account, DEFAULT_EXCLUSIONS)) = key.rsplit_once(':') {
            let on = match value {
                "on" | "true" | "yes" => true,
                "off" | "false" | "no" => false,
                _ => {
                    return Err(Error::InvalidArgument(format!(
                        "\"{value}\" is not valid for {DEFAULT_EXCLUSIONS}: use on or off."
                    )));
                }
            };
            let id = AccountId::new(account);
            // Changing them remounts; uploads pending are no reason to refuse
            // a setting, they upload after the remount.
            self.set_default_exclusions(&id, on, true).await?;
            return self.config_get(Some(key)).await;
        }
        let mut st = self.state.lock().await;
        // Mounted accounts whose cache options changed.
        let mut remount: Vec<usize> = Vec::new();
        let result = if let Some((account, setting)) = key.rsplit_once(':') {
            let i = st.resolve(&AccountId::new(account))?;
            remount.push(i);
            let r = &mut st.config.accounts[i];
            match setting {
                CACHE_LIMIT => {
                    let bytes = parse_size(value).ok_or_else(|| bad_size(value))?;
                    r.account.cache_limit = bytes;
                    entry(
                        format!("{}:{CACHE_LIMIT}", r.account.display_name),
                        format_size(bytes),
                    )
                }
                CACHE_MAX_AGE => {
                    r.cache_max_age_secs = parse_age(value)?;
                    entry(
                        format!("{}:{CACHE_MAX_AGE}", r.account.display_name),
                        age_text(r.cache_max_age_secs),
                    )
                }
                _ => return Err(unknown_key(key)),
            }
        } else {
            let s = &mut st.config.settings;
            match key {
                CACHE_LIMIT => {
                    s.default_cache_limit = parse_size(value).ok_or_else(|| bad_size(value))?;
                    entry(key, format_size(s.default_cache_limit))
                }
                CACHE_MAX_AGE => {
                    s.cache_max_age_secs = parse_age(value)?;
                    let text = age_text(s.cache_max_age_secs);
                    // Accounts without their own age follow this one.
                    remount.extend(
                        st.config
                            .accounts
                            .iter()
                            .enumerate()
                            .filter(|(_, r)| r.cache_max_age_secs.is_none())
                            .map(|(i, _)| i),
                    );
                    entry(key, text)
                }
                QUIT_WITH_PENDING => {
                    s.quit_with_pending = QuitBehavior::parse(value).ok_or_else(|| {
                        let options: Vec<_> =
                            QuitBehavior::ALL.iter().map(|b| b.as_str()).collect();
                        Error::InvalidArgument(format!(
                            "\"{value}\" is not valid for {QUIT_WITH_PENDING}. Choose one of: {}.",
                            options.join(", ")
                        ))
                    })?;
                    entry(key, s.quit_with_pending.as_str())
                }
                _ => return Err(unknown_key(key)),
            }
        };
        self.save(&st)?;
        for i in remount {
            // `remount_hidden` skips accounts that are not mounted.
            if let Err(e) = self.remount_hidden(&mut st, i).await {
                tracing::warn!(error = %e, "cannot apply the new cache settings");
                return Err(Error::Rclone(format!(
                    "The setting is saved but could not be applied yet ({e}); it applies at the next mount."
                )));
            }
        }
        Ok(vec![result])
    }
}
