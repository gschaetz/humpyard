//! Where `humpyard up` keeps things: the state directory, the per-run directory and the configuration the gateway
//! child loads (ADR 0039).

use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// The state directory: `--state-dir`, else `$XDG_CONFIG_HOME/humpyard`, else `$HOME/.config/humpyard`.
///
/// # Errors
/// Neither a directory flag nor a home directory is available.
pub fn state_dir(
    flag: Option<&Path>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<PathBuf, String> {
    if let Some(dir) = flag {
        return Ok(dir.to_path_buf());
    }
    let from = |name: &str| env(name).filter(|v| !v.is_empty());
    if let Some(base) = from("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(base).join("humpyard"));
    }
    from("HOME")
        .map(|home| PathBuf::from(home).join(".config").join("humpyard"))
        .ok_or_else(|| "no home directory is known; name one with --state-dir".to_string())
}

/// The directory of one run, mode `0700`, removed when it is dropped. It holds only the configuration the gateway
/// child loads, which carries the hash of the run's admin credential and never the credential.
#[derive(Debug)]
pub struct RunDir {
    path: PathBuf,
}

impl RunDir {
    /// Removes what an earlier run left (a crash skips the drop) and makes this run's directory.
    ///
    /// # Errors
    /// The directory cannot be made.
    pub fn create(state: &Path) -> Result<Self, String> {
        let parent = state.join("run");
        // A second `up` on the same state directory would fail on its ports before reaching this.
        let _ = std::fs::remove_dir_all(&parent);
        let path = parent.join(std::process::id().to_string());
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)
            .map_err(|e| format!("cannot make {}: {e}", path.display()))?;
        Ok(Self { path })
    }

    /// Writes the configuration the gateway child loads, readable only by this user.
    ///
    /// # Errors
    /// The file cannot be written.
    pub fn write_config(&self, text: &str) -> Result<PathBuf, String> {
        use std::io::Write;
        let path = self.path.join("config.toml");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok(path)
    }
}

impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The user's configuration plus what a run needs added: the run's admin credential (`hash`), a ledger in the state
/// directory when the file names none (so the page has history across restarts), and `listen` when the flag gave one.
/// A file that already has an `[admin]` section is refused, not merged.
///
/// # Errors
/// The text is not valid TOML, has an `[admin]` section, or the result would not load.
pub fn run_config(
    user: &str,
    hash: &str,
    listen: Option<&str>,
    ledger: &Path,
) -> Result<String, String> {
    use toml_edit::{DocumentMut, value};
    let mut doc: DocumentMut = user
        .parse()
        .map_err(|e| format!("the configuration is not valid TOML: {e}"))?;
    if doc.contains_key("admin") {
        return Err(
            "the configuration has an [admin] section, and `humpyard up` makes its own admin credential for each \
             run rather than edit yours. Run `humpyard serve` and `humpyard ui` separately for a setup with its own \
             admin credentials (docs/admin-gui.md)"
                .into(),
        );
    }
    if let Some(listen) = listen {
        doc["listen"] = value(listen);
    }
    if !doc.contains_key("ledger") {
        doc["ledger"]["path"] = value(ledger.display().to_string());
    }
    let block = crate::admin::credential_block("up", &["read".to_string()], None, hash)?;
    Ok(format!("{doc}\n{block}"))
}

/// The address the gateway will listen on, from the (possibly overridden) configuration text.
///
/// # Errors
/// The text is not TOML or has no usable `listen`.
pub fn listen_of(text: &str) -> Result<std::net::SocketAddr, String> {
    let table: toml::Table = text
        .parse()
        .map_err(|e| format!("the configuration is not valid TOML: {e}"))?;
    let listen = table
        .get("listen")
        .and_then(toml::Value::as_str)
        .ok_or("the configuration has no `listen` address")?;
    listen
        .parse()
        .map_err(|e| format!("`listen = {listen}` is not an address like 127.0.0.1:8080: {e}"))
}

/// What a configuration says about one provider, for the page's key list and the live check of a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderFacts {
    /// The environment variable its key is read from (`api_key_env`).
    pub variable: String,
    pub base_url: Option<String>,
    /// A model one of its endpoints uses, for a small test request.
    pub model: Option<String>,
}

/// Each provider the configuration names, by provider name.
#[must_use]
pub fn provider_facts(text: &str) -> std::collections::BTreeMap<String, ProviderFacts> {
    let Ok(table) = text.parse::<toml::Table>() else {
        return std::collections::BTreeMap::new();
    };
    let text_of =
        |value: Option<&toml::Value>| value.and_then(toml::Value::as_str).map(str::to_string);
    let model_of = |provider: &str| -> Option<String> {
        table
            .get("targets")?
            .as_table()?
            .values()
            .filter_map(|target| target.get("endpoints")?.as_array())
            .flatten()
            .find(|endpoint| {
                endpoint.get("provider").and_then(toml::Value::as_str) == Some(provider)
            })
            .and_then(|endpoint| text_of(endpoint.get("model")))
    };
    table
        .get("providers")
        .and_then(toml::Value::as_table)
        .map(|providers| {
            providers
                .iter()
                .filter_map(|(name, body)| {
                    Some((
                        name.clone(),
                        ProviderFacts {
                            variable: text_of(body.get("api_key_env"))?,
                            base_url: text_of(body.get("base_url")),
                            model: model_of(name),
                        },
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The names a client may ask for as `model`: every route and every target the configuration has.
#[must_use]
pub fn route_names(text: &str) -> Vec<String> {
    let Ok(table) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for section in ["routes", "targets"] {
        if let Some(entries) = table.get(section).and_then(toml::Value::as_table) {
            names.extend(entries.keys().cloned());
        }
    }
    names.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "listen = \"127.0.0.1:8080\"\n[providers.groq]\nbase_url = \"https://api.groq.com/openai/v1\"\napi_key_env = \"GROQ_API_KEY\"\n[targets.fast]\nendpoints = [{ provider = \"groq\", model = \"m\" }]\n";

    #[test]
    fn the_state_directory_follows_the_flag_then_xdg_then_home() {
        let env = |name: &str| match name {
            "XDG_CONFIG_HOME" => Some("/x".to_string()),
            "HOME" => Some("/h".to_string()),
            _ => None,
        };
        assert_eq!(
            state_dir(Some(Path::new("/f")), &env).unwrap(),
            Path::new("/f")
        );
        assert_eq!(state_dir(None, &env).unwrap(), Path::new("/x/humpyard"));
        let home = |name: &str| (name == "HOME").then(|| "/h".to_string());
        assert_eq!(
            state_dir(None, &home).unwrap(),
            Path::new("/h/.config/humpyard")
        );
        assert!(state_dir(None, &|_| None).is_err());
    }

    #[test]
    fn an_admin_section_is_refused_not_merged() {
        let text = format!(
            "{USER}[admin.credentials.mine]\nsha256 = \"sha256:{}\"\nscopes = [\"read\"]\n",
            "0".repeat(64)
        );
        let error = run_config(
            &text,
            &format!("sha256:{}", "1".repeat(64)),
            None,
            Path::new("/s/l.db"),
        )
        .unwrap_err();
        assert!(error.contains("[admin]"), "{error}");
    }

    #[test]
    fn the_run_config_adds_the_credential_and_a_ledger_and_keeps_the_rest() {
        let hash = format!("sha256:{}", "1".repeat(64));
        let text = run_config(
            USER,
            &hash,
            Some("127.0.0.1:9090"),
            Path::new("/s/ledger.db"),
        )
        .unwrap();
        assert!(text.contains("[admin.credentials.up]"), "{text}");
        assert!(text.contains(&hash));
        assert!(text.contains("/s/ledger.db"));
        assert_eq!(listen_of(&text).unwrap().port(), 9090);
        assert!(text.contains("[providers.groq]"));
    }

    #[test]
    fn a_ledger_the_user_chose_is_kept() {
        let hash = format!("sha256:{}", "1".repeat(64));
        let user = format!("{USER}[ledger]\npath = \"mine.db\"\n");
        let text = run_config(&user, &hash, None, Path::new("/s/ledger.db")).unwrap();
        assert!(text.contains("mine.db") && !text.contains("/s/ledger.db"));
    }

    #[test]
    fn what_a_configuration_says_about_a_provider_is_found() {
        let facts = provider_facts(USER);
        assert_eq!(facts["groq"].variable, "GROQ_API_KEY");
        assert_eq!(
            facts["groq"].base_url.as_deref(),
            Some("https://api.groq.com/openai/v1")
        );
        assert_eq!(facts["groq"].model.as_deref(), Some("m"));
        assert!(provider_facts("not toml [").is_empty());
    }

    #[test]
    fn the_names_a_client_may_ask_for_are_found() {
        assert_eq!(route_names(USER), vec!["fast".to_string()]);
        let with_route = format!("{USER}[routes.deep]\nmode = \"pool\"\n");
        assert_eq!(
            route_names(&with_route),
            vec!["deep".to_string(), "fast".to_string()]
        );
    }
}
