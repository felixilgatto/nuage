//! `nuage config get/set`.
//!
//! | Key | Value |
//! |---|---|
//! | `cache-limit` | cache limit of new accounts (`20G`) |
//! | `<account>:cache-limit` | cache limit of one account |
//! | `cache-max-age` | maximum age of cached files (`30d`) |
//! | `quit-with-pending` | `finish-in-background`, `ask` or `quit-now` |
//!
//! Cache settings apply the next time an account is mounted.

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
            format!("{}:{DEFAULT_EXCLUSIONS}", r.account.display_name),
            on_off(r.default_exclusions),
        ));
    }
    entries
}

fn unknown_key(key: &str) -> Error {
    Error::InvalidArgument(format!(
        "Unknown setting \"{key}\". Settings: {CACHE_LIMIT}, <account>:{CACHE_LIMIT}, <account>:{DEFAULT_EXCLUSIONS}, {CACHE_MAX_AGE}, {QUIT_WITH_PENDING}."
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
        let result = if let Some((account, setting)) = key.rsplit_once(':') {
            if setting != CACHE_LIMIT {
                return Err(unknown_key(key));
            }
            let bytes = parse_size(value).ok_or_else(|| bad_size(value))?;
            let i = st.resolve(&AccountId::new(account))?;
            let a = &mut st.config.accounts[i].account;
            a.cache_limit = bytes;
            entry(
                format!("{}:{CACHE_LIMIT}", a.display_name),
                format_size(bytes),
            )
        } else {
            let s = &mut st.config.settings;
            match key {
                CACHE_LIMIT => {
                    s.default_cache_limit = parse_size(value).ok_or_else(|| bad_size(value))?;
                    entry(key, format_size(s.default_cache_limit))
                }
                CACHE_MAX_AGE => {
                    if value == "default" {
                        s.cache_max_age_secs = None;
                        entry(key, "default")
                    } else {
                        let secs = parse_duration(value).ok_or_else(|| {
                            Error::InvalidArgument(format!(
                                "\"{value}\" is not a duration. Write it like 30d, 12h or 2w, or `default`."
                            ))
                        })?;
                        s.cache_max_age_secs = Some(secs);
                        entry(key, format_duration(secs))
                    }
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
        Ok(vec![result])
    }
}
