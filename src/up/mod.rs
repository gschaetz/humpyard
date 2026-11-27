//! `humpyard up`: one command from install to a working, visible gateway on one machine (ADR 0039, spec:
//! `single-machine-up`).
//!
//! `up` is a supervisor. It runs `humpyard serve` as a child process, so the process that handles model requests
//! serves no page (ADR 0032), and it serves the admin GUI itself with the same router `humpyard ui` uses, adding the
//! routes under `/local` that the first-run screens need. The admin credential is made for each run: the gateway is
//! given only its hash, in a file in a private directory that is removed on exit, and the GUI is given the credential
//! in memory. Provider keys saved in the page live in one private file that only this module reads, and reach the
//! gateway as environment variables, so `serve` and its loader are unchanged (ADR 0004). Only `main` may use this
//! module.

mod local;
mod state;

use std::future::Future;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use std::collections::BTreeMap;

use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::keystore::{KeyStore, Secret, overlay};

pub use state::{ProviderFacts, listen_of, provider_facts, route_names, run_config, state_dir};

/// What the user asked for.
pub struct Settings {
    /// `--state-dir`; otherwise the user's config directory.
    pub state_dir: Option<PathBuf>,
    /// `--listen`: where the gateway listens; otherwise the configuration's own `listen`, else `127.0.0.1:8080`.
    pub listen: Option<String>,
    /// `--ui-listen`: where the GUI is served.
    pub ui_listen: String,
    /// `--allow-host`: a name the GUI answers to besides loopback; needed (and key entry is switched off) when the
    /// GUI listens beyond this machine.
    pub allow_hosts: Vec<String>,
    /// `--custom NAME,BASE_URL,KEY_ENV,MODEL`: an OpenAI-compatible endpoint besides the built-in providers, used when
    /// a first configuration is written.
    pub custom: Vec<String>,
    /// Open the browser on the signed-in link (`--no-open` turns it off).
    pub open_browser: bool,
    /// The binary to run as the gateway child; this binary when absent. Tests name the built one.
    pub exe: Option<PathBuf>,
}

/// The environment `up` reads: the variable a key may already be set in, and `HOME`.
pub type Env = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The name the run's admin credential is given to the GUI router under. It is never a real environment variable.
const CREDENTIAL_NAME: &str = "HUMPYARD_UP_ADMIN";
/// How long the gateway may take to start listening.
const READY_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a stopping gateway may take to finish its work before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(15);

/// The gateway child: where it is, and the process itself.
pub(crate) struct Gateway {
    exe: PathBuf,
    cwd: PathBuf,
    pub(crate) addr: SocketAddr,
    child: Mutex<Option<Child>>,
}

impl Gateway {
    fn new(exe: PathBuf, cwd: PathBuf, addr: SocketAddr) -> Self {
        Self {
            exe,
            cwd,
            addr,
            child: Mutex::new(None),
        }
    }

    /// Starts `serve` on `config` with `extra` added to its environment, and returns once it is listening.
    pub(crate) async fn start(
        &self,
        config: &Path,
        extra: &[(String, Secret)],
    ) -> Result<(), String> {
        let mut guard = self.child.lock().await;
        if guard.is_some() {
            return Err("the gateway is already running".into());
        }
        let mut command = Command::new(&self.exe);
        command
            .arg("serve")
            .arg("--config")
            .arg(config)
            .current_dir(&self.cwd)
            .kill_on_drop(true);
        for (name, value) in extra {
            command.env(name, value.expose());
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("cannot start the gateway ({}): {e}", self.exe.display()))?;
        let started = std::time::Instant::now();
        loop {
            if tokio::net::TcpStream::connect(self.addr).await.is_ok() {
                *guard = Some(child);
                return Ok(());
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!(
                    "the gateway exited ({status}) before it was listening"
                ));
            }
            if started.elapsed() > READY_TIMEOUT {
                let _ = child.start_kill();
                return Err(format!(
                    "the gateway did not listen on {} in time",
                    self.addr
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Asks the gateway to finish and waits for it, killing it only after the grace period.
    pub(crate) async fn stop(&self) {
        let Some(mut child) = self.child.lock().await.take() else {
            return;
        };
        if let Some(pid) = child.id() {
            // The same signal `serve` already handles to flush its ledger and drain; no library call needed.
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
        }
        if tokio::time::timeout(STOP_GRACE, child.wait())
            .await
            .is_err()
        {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }

    /// Why the gateway is no longer running, when it has stopped without being asked to.
    async fn exited(&self) -> Option<String> {
        let mut guard = self.child.lock().await;
        let status = guard.as_mut()?.try_wait().ok().flatten()?;
        *guard = None;
        Some(format!("the gateway stopped ({status})"))
    }

    pub(crate) async fn running(&self) -> bool {
        self.child.lock().await.is_some()
    }
}

/// What the page's `/local` routes and the supervisor share.
pub(crate) struct Core {
    pub(crate) state: PathBuf,
    pub(crate) store: KeyStore,
    pub(crate) gateway: Gateway,
    run: state::RunDir,
    hash: String,
    listen_override: Option<String>,
    custom: Vec<String>,
    pub(crate) env: Env,
    /// Keys entered in the page for this run only. They live in memory and are never written anywhere.
    pub(crate) session: parking_lot::Mutex<BTreeMap<String, Secret>>,
    /// Key entry is on only when the page is served on this machine alone.
    pub(crate) key_entry: bool,
}

impl Core {
    pub(crate) fn config_path(&self) -> PathBuf {
        self.state.join("config.toml")
    }

    pub(crate) fn has_config(&self) -> bool {
        self.config_path().exists()
    }

    /// Every key available to this run, by variable name: the file's, then those entered for this run only. The real
    /// environment is not in it; it always wins, and a child inherits it.
    pub(crate) fn held_keys(&self) -> Result<BTreeMap<String, Secret>, String> {
        let mut keys = self.store.load().map_err(|e| e.to_string())?;
        keys.extend(self.session.lock().clone());
        Ok(keys)
    }

    /// The keys to add to a child's environment: those the real environment does not already set.
    pub(crate) fn extra_environment(&self) -> Result<Vec<(String, Secret)>, String> {
        Ok(overlay(&self.held_keys()?, &*self.env))
    }

    /// A reader of a variable as the gateway child will see it: the real environment, else a key entered for this
    /// run, else the file. It is a snapshot, so a later change is not seen by it.
    pub(crate) fn lookup(&self) -> Result<impl Fn(&str) -> Option<String> + use<>, String> {
        let held = self.held_keys()?;
        let env = Arc::clone(&self.env);
        Ok(move |name: &str| {
            env(name)
                .filter(|v| !v.is_empty())
                .or_else(|| held.get(name).map(|s| s.expose().to_string()))
        })
    }

    /// The providers of the user's configuration that have no key anywhere, by name and variable.
    pub(crate) fn missing_keys(&self) -> Result<Vec<(String, String)>, String> {
        let Ok(text) = std::fs::read_to_string(self.config_path()) else {
            return Ok(Vec::new());
        };
        let lookup = self.lookup()?;
        Ok(provider_facts(&text)
            .into_iter()
            .filter(|(_, facts)| lookup(&facts.variable).is_none())
            .map(|(name, facts)| (name, facts.variable))
            .collect())
    }

    /// Writes this run's configuration from the user's and starts the gateway on it.
    pub(crate) async fn start_gateway(&self) -> Result<(), String> {
        let user = std::fs::read_to_string(self.config_path())
            .map_err(|e| format!("cannot read {}: {e}", self.config_path().display()))?;
        let text = run_config(
            &user,
            &self.hash,
            self.listen_override.as_deref(),
            &self.state.join("ledger.db"),
        )?;
        let path = self.run.write_config(&text)?;
        let extra = self.extra_environment()?;
        self.gateway.start(&path, &extra).await
    }

    /// Stops the gateway and starts it again, so a changed key takes effect.
    pub(crate) async fn restart_gateway(&self) -> Result<(), String> {
        self.gateway.stop().await;
        self.start_gateway().await
    }
}

fn say(out: &mut dyn Write, line: &str) -> Result<(), String> {
    writeln!(out, "{line}").map_err(|e| e.to_string())
}

/// Opens `url` in the default browser. A failure is not an error: the address is printed instead.
fn open_browser(url: &str) -> bool {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A configuration for the providers whose keys are set, when there is none yet. Returns whether one was written.
pub(crate) async fn first_config(core: &Core, out: &mut dyn Write) -> Result<bool, String> {
    let with_saved = core.lookup()?;
    let providers: Vec<String> = crate::init::PRESETS
        .iter()
        .filter(|p| with_saved(p.key_env).is_some())
        .map(|p| p.name.to_string())
        .collect();
    let mut custom = Vec::new();
    for text in &core.custom {
        let pick = crate::init::parse_custom(text)?;
        if with_saved(&pick.key_env).is_some() {
            custom.push(text.clone());
        }
    }
    if providers.is_empty() && custom.is_empty() {
        return Ok(false);
    }
    let options = crate::init::InitOptions {
        providers,
        custom,
        out: core.config_path(),
        listen: core
            .listen_override
            .clone()
            .unwrap_or_else(|| crate::init::default_listen().to_string()),
        no_proof: false,
    };
    let mut config_sink = Vec::new();
    crate::init::run(options, &with_saved, None, out, &mut config_sink).await?;
    Ok(core.has_config())
}

/// Where the gateway will listen: `--listen`, else the configuration's own, else the default. It must be this
/// machine's and free, and a refusal names the flag that changes it.
fn gateway_address(settings: &Settings, state: &Path) -> Result<SocketAddr, String> {
    let from_file = std::fs::read_to_string(state.join("config.toml"));
    let addr: SocketAddr = match (&settings.listen, from_file) {
        (Some(listen), _) => listen.parse().map_err(|e| {
            format!("`--listen {listen}` is not an address like 127.0.0.1:8080: {e}")
        })?,
        (None, Ok(text)) => listen_of(&text)?,
        (None, Err(_)) => crate::init::default_listen()
            .parse()
            .map_err(|e| format!("{e}"))?,
    };
    if !addr.ip().is_loopback() {
        return Err(format!(
            "{addr} is not a loopback address. `humpyard up` is for one machine; use `humpyard serve` with keys and \
             budgets for a gateway others can reach"
        ));
    }
    if std::net::TcpListener::bind(addr).is_err() {
        return Err(format!(
            "{addr} is already in use for the gateway. Choose another with --listen"
        ));
    }
    Ok(addr)
}

/// Tells the user where things are, and opens the signed-in link or prints it.
async fn announce(
    core: &Core,
    settings: &Settings,
    ui_bound: SocketAddr,
    token: &str,
    out: &mut dyn Write,
) -> Result<(), String> {
    say(
        out,
        &format!("humpyard up: the GUI is at http://{ui_bound}"),
    )?;
    if core.gateway.running().await {
        say(
            out,
            &format!(
                "humpyard up: the gateway is at http://{}",
                core.gateway.addr
            ),
        )?;
    }
    say(
        out,
        &format!("humpyard up: state is in {}", core.state.display()),
    )?;
    if !crate::ui::GUI_EMBEDDED {
        say(
            out,
            "this build has no GUI files: the page says how to build them",
        )?;
    }
    let link = format!("http://{ui_bound}/#token={token}");
    if settings.open_browser && open_browser(&link) {
        say(out, "humpyard up: opened the GUI in your browser")?;
    } else {
        say(
            out,
            &format!("humpyard up: open this link to sign in: {link}"),
        )?;
    }
    out.flush().map_err(|e| e.to_string())
}

/// Checks the settings, starts the gateway and the GUI, tells the user, and serves until `shutdown` completes or
/// the gateway stops.
///
/// # Errors
/// A setting that is not safe, an address that is taken, a configuration `up` will not edit, or a gateway that does
/// not start.
pub async fn run(
    settings: &Settings,
    env: Env,
    out: &mut dyn Write,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let state = state_dir(settings.state_dir.as_deref(), &*env)?;
    let store = KeyStore::open(&state).map_err(|e| e.to_string())?;

    let ui_addr: SocketAddr = settings.ui_listen.parse().map_err(|e| {
        format!(
            "`--ui-listen {}` is not an address like 127.0.0.1:8787: {e}",
            settings.ui_listen
        )
    })?;
    let key_entry = ui_addr.ip().is_loopback();
    let listener = tokio::net::TcpListener::bind(ui_addr).await.map_err(|e| {
        format!("cannot listen on {ui_addr} for the GUI: {e}. Choose another with --ui-listen")
    })?;
    let ui_bound = listener.local_addr().map_err(|e| e.to_string())?;

    let gateway_addr = gateway_address(settings, &state)?;

    let (credential, hash) =
        crate::auth::generate_key().map_err(|e| format!("cannot gather randomness: {e}"))?;
    let exe = match &settings.exe {
        Some(exe) => exe.clone(),
        None => std::env::current_exe().map_err(|e| format!("cannot find this program: {e}"))?,
    };
    let core = Arc::new(Core {
        run: state::RunDir::create(&state)?,
        gateway: Gateway::new(exe, state.clone(), gateway_addr),
        store,
        state,
        hash,
        listen_override: settings.listen.clone(),
        custom: settings.custom.clone(),
        env,
        session: parking_lot::Mutex::new(BTreeMap::new()),
        key_entry,
    });

    if !core.has_config() {
        say(
            out,
            "humpyard up: no configuration yet; looking for provider keys",
        )?;
        if !first_config(&core, out).await? {
            say(
                out,
                "humpyard up: no provider key found. Add one in the page, or set GROQ_API_KEY, OPENROUTER_API_KEY or NVIDIA_API_KEY and run this again",
            )?;
        }
    }
    if core.has_config() {
        let missing = core.missing_keys()?;
        if missing.is_empty() {
            core.start_gateway().await?;
        } else {
            let names: Vec<String> = missing
                .iter()
                .map(|(provider, variable)| format!("{provider} (${variable})"))
                .collect();
            say(
                out,
                &format!(
                    "humpyard up: no key for {}. The gateway is not started; add a key in the page",
                    names.join(", ")
                ),
            )?;
        }
    }

    let ui_settings = crate::ui::Settings {
        url: format!("http://{gateway_addr}"),
        listen: ui_bound.to_string(),
        credential_env: Some(CREDENTIAL_NAME.to_string()),
        allow_remote_with_credential: !ui_bound.ip().is_loopback(),
        allow_hosts: settings.allow_hosts.clone(),
        allow_insecure_upstream: false,
    };
    let credential_value = credential;
    let credential_env =
        move |name: &str| (name == CREDENTIAL_NAME).then(|| credential_value.clone());
    let prepared = crate::ui::prepare_with_local(
        &ui_settings,
        &credential_env,
        Arc::new(local::UpLocal::new(Arc::clone(&core))),
    )?;
    let token = prepared.token.clone().ok_or("no access token was made")?;

    announce(&core, settings, ui_bound, &token, out).await?;

    let watcher = Arc::clone(&core);
    let gateway_stopped = async move {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Some(reason) = watcher.gateway.exited().await {
                return reason;
            }
        }
    };
    let result = tokio::select! {
        served = axum::serve(listener, prepared.router) => served.map_err(|e| e.to_string()),
        () = shutdown => Ok(()),
        reason = gateway_stopped => Err(reason),
    };
    core.gateway.stop().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_credential_name_is_not_an_environment_variable_anyone_sets() {
        assert!(CREDENTIAL_NAME.starts_with("HUMPYARD_UP_"));
    }
}
