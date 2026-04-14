use std::collections::BTreeMap;
use std::fs::{File, create_dir, create_dir_all, read_to_string, remove_file, write};
use std::path::{PathBuf, absolute};
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow, bail, ensure};
use clap::{Parser, Subcommand};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_yml::{Mapping, Value};
use url::Url;
use which::which;

static CLIENT: LazyLock<Client> = LazyLock::new(Client::new);

#[derive(Serialize, Deserialize)]
struct MihuConfig {
    mihomo_path: PathBuf,
    external_control_url: String,
    dashboard_url: String,
    current_sub: String,
    default_sub: String,
    subs: BTreeMap<String, String>,
}

impl MihuConfig {
    fn load() -> Result<Self> {
        let config_path = dirs::config_dir().unwrap().join("mihu/config.yaml");
        ensure!(config_path.exists(), "config does not exist, please run \"mihu init\" first");
        Ok(serde_yml::from_reader(File::open(config_path)?)?)
    }

    fn save(&self) -> Result<()> {
        let config_path = dirs::config_dir().unwrap().join("mihu/config.yaml");
        serde_yml::to_writer(File::create(config_path)?, self)?;
        Ok(())
    }

    fn reload_mihomo(&self, name: &str) -> Result<()> {
        let sub_path = get_sub_path(name, false);
        let sub_override_path = get_sub_path(name, true);
        let global_override_path = dirs::config_dir().unwrap().join("mihu/global_override.yaml");

        let mut sub_config: Value = serde_yml::from_reader(File::open(sub_path)?)?;
        let global_override: Value = serde_yml::from_reader(File::open(global_override_path)?)?;

        let sub_override: Option<Value> = if sub_override_path.exists() {
            Some(serde_yml::from_reader(File::open(sub_override_path)?)?)
        } else {
            None
        };

        let Some(sub_config) = sub_config.as_mapping_mut() else {
            bail!("subscription config is malformed");
        };
        let Some(global_override) = global_override.as_mapping() else {
            bail!("global override is malformed");
        };

        merge(sub_config, global_override);

        if let Some(sub_override) = sub_override {
            let Some(sub_override) = sub_override.as_mapping() else {
                bail!("subscription override is malformed");
            };
            merge(sub_config, sub_override);
        }

        serde_yml::to_writer(File::create(&self.mihomo_path)?, sub_config)?;

        let endpoint = format!("{}/configs", self.external_control_url.trim_end_matches('/'));
        CLIENT
            .put(endpoint)
            .body(r#"{"path": "", "payload": ""}"#)
            .send()
            .map_err(|_| anyhow!("cannot reload mihomo config"))?;

        Ok(())
    }
}

impl Default for MihuConfig {
    fn default() -> Self {
        Self {
            mihomo_path: dirs::config_dir().unwrap().join("mihomo/config.yaml"),
            external_control_url: "http://127.0.0.1:9090/".into(),
            dashboard_url: "https://board.zash.run.place/".into(),
            default_sub: String::new(),
            current_sub: String::new(),
            subs: BTreeMap::new(),
        }
    }
}

fn merge(target: &mut Mapping, ovrd: &Mapping) {
    fn trim_wrap(key: &str) -> &str {
        key.strip_prefix('<').and_then(|s| s.strip_suffix('>')).unwrap_or(key)
    }

    for (key, value) in ovrd {
        let Some(key) = key.as_str() else { continue };
        match value {
            Value::Mapping(map) => {
                if let Some(stripped) = key.strip_suffix('!') {
                    let k = trim_wrap(stripped);
                    target.insert(Value::String(k.to_owned()), value.clone());
                } else {
                    let k = trim_wrap(key);
                    if target.get(k).is_none_or(|v| !v.is_mapping()) {
                        target.insert(Value::String(k.to_owned()), Value::Mapping(Mapping::new()));
                    }
                    merge(target[k].as_mapping_mut().unwrap(), map);
                }
            }
            Value::Sequence(seq) => {
                if let Some(stripped) = key.strip_prefix('+') {
                    let k = trim_wrap(stripped);
                    if target.get(k).is_none_or(|v| !v.is_sequence()) {
                        target.insert(Value::String(k.to_owned()), Value::Sequence(Vec::new()));
                    }
                    let mut list = seq.clone();
                    list.extend_from_slice(target[k].as_sequence().unwrap());
                    target[k] = Value::Sequence(list);
                } else if let Some(stripped) = key.strip_suffix('+') {
                    let k = trim_wrap(stripped);
                    if target.get(k).is_none_or(|v| !v.is_sequence()) {
                        target.insert(Value::String(k.to_owned()), Value::Sequence(Vec::new()));
                    }
                    target[k].as_sequence_mut().unwrap().extend_from_slice(seq);
                } else {
                    let k = trim_wrap(key);
                    target.insert(Value::String(k.to_owned()), value.clone());
                }
            }
            _ => {
                let k = trim_wrap(key);
                target.insert(Value::String(k.to_owned()), value.clone());
            }
        }
    }
}

fn get_sub_path(name: &str, get_override: bool) -> PathBuf {
    let subfolder = if get_override { "overrides" } else { "subscriptions" };
    dirs::config_dir().unwrap().join("mihu").join(subfolder).join(name).with_added_extension("yaml")
}

fn fetch_sub_config(url: &str) -> Result<String> {
    let response = CLIENT.get(url).header("user-agent", "clash.meta").send()?;
    ensure!(response.status().is_success(), "cannot fetch subscription: {}", response.status());
    let content = response.text()?;
    ensure!(serde_yml::from_str::<Value>(&content)?.is_mapping(), "fetched config is malformed");
    Ok(content)
}

// CLI stuff

/// A lightweight config manager for Clash & Mihomo
#[derive(Parser)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Add a new subscription, or edit an existing one
    Sub {
        /// Subscription name
        name: String,
        /// Subscription URL
        url: String,
        /// Switch to it
        #[arg(short, long)]
        switch: bool,
        /// Set it as default
        #[arg(short, long)]
        default: bool,
    },
    /// Remove a subscription
    #[command(visible_alias = "rm")]
    Remove {
        /// Subscription name
        name: String,
        /// Also remove its override profile
        #[arg(short = 'o', long = "override")]
        remove_override: bool,
    },
    /// Switch subscription
    #[command(visible_alias = "sw")]
    Switch {
        /// Subscription name, empty for default
        name: Option<String>,
        /// Update it
        #[arg(short, long)]
        update: bool,
        /// Set it as default
        #[arg(short, long)]
        default: bool,
    },
    /// Update subscriptions
    #[command(visible_alias = "up")]
    Update {
        /// Subscriptions to update, empty for current
        names: Vec<String>,
        /// Update all, ignore names
        #[arg(short, long)]
        all: bool,
    },
    /// Edit override profiles
    Edit {
        /// Name of the subscription, empty for global override
        name: Option<String>,
        /// Specify editor, empty for environment variable "EDITOR"
        #[arg(short, long)]
        editor: Option<String>,
    },
    /// Open web dashboard in browser
    Dash {
        /// Edit address
        #[arg(short, long)]
        edit: Option<String>,
    },
    /// Print information about current configuration
    Info {
        /// Print detailed information
        #[arg(short, long, exclusive = true)]
        verbose: bool,
        /// Print raw configuration in YAML
        #[arg(short, long, exclusive = true)]
        raw: bool,
        /// Print current mihomo configuration
        #[arg(short, long, exclusive = true)]
        mihomo: bool,
    },
    /// Initialize the software
    Init {
        /// Path to Mihomo's configuration
        #[arg(short = 'c', long = "config")]
        mihomo_config: Option<PathBuf>,
        /// Mihomo's external control endpoint URL
        #[arg(short = 'e', long = "extctl")]
        extctl_url: Option<String>,
        /// Dashboard URL
        #[arg(short = 'd', long = "dash")]
        dashboard_url: Option<String>,
    },
}

fn add_sub(name: String, url: String, switch: bool, set_default: bool) -> Result<()> {
    ensure!(Url::parse(&url).is_ok(), "invalid URL");
    let mut config = MihuConfig::load()?;
    let sub_path = get_sub_path(&name, false);
    write(&sub_path, fetch_sub_config(&url)?)?;
    if switch {
        config.current_sub = name.clone();
        config.reload_mihomo(&name)?;
    }
    if set_default {
        config.default_sub = name.clone();
    }
    config.subs.insert(name, url);
    config.save()?;
    Ok(())
}

fn remove_sub(name: String, remove_override: bool) -> Result<()> {
    let mut config = MihuConfig::load()?;
    config.subs.remove(&name);

    let mut need_reload = false;
    let fallback = config.subs.keys().next().cloned().unwrap_or_default();
    if config.current_sub == name {
        config.current_sub = fallback.clone();
        need_reload = true;
    }
    if config.default_sub == name {
        config.default_sub = fallback;
    }

    config.save()?;
    let sub_path = get_sub_path(&name, false);
    if sub_path.exists() {
        remove_file(sub_path)?;
    }
    if remove_override {
        let ovrd_path = get_sub_path(&name, true);
        if ovrd_path.exists() {
            remove_file(ovrd_path)?;
        }
    }

    if need_reload && !config.current_sub.is_empty() {
        config.reload_mihomo(&config.current_sub)?;
    }
    Ok(())
}

fn switch_sub(name: Option<String>, update: bool, set_default: bool) -> Result<()> {
    let mut config = MihuConfig::load()?;

    ensure!(name.is_some() || !config.default_sub.is_empty(), "default subscription is not set");
    let name = name.unwrap_or(config.default_sub.clone());

    ensure!(config.subs.contains_key(&name), "subscription \"{name}\" does not exist");
    let sub_path = get_sub_path(&name, false);
    if update {
        let url = &config.subs[&name];
        write(&sub_path, fetch_sub_config(url)?)?;
    }
    config.reload_mihomo(&name)?;
    if set_default {
        config.default_sub = name.clone();
    }
    config.current_sub = name;
    config.save()?;
    Ok(())
}

fn update_subs(names: Vec<String>, update_all: bool) -> Result<()> {
    let config = MihuConfig::load()?;

    let names: Vec<_> = if update_all {
        config.subs.keys().collect()
    } else if names.is_empty() {
        vec![&config.current_sub]
    } else {
        names.iter().collect()
    };
    let need_reload = update_all || names.is_empty() || names.contains(&&config.current_sub);

    names
        .into_par_iter()
        .map(|name| {
            ensure!(config.subs.contains_key(name), "subscription \"{name}\" does not exist");
            let sub_path = get_sub_path(name, false);
            let url = &config.subs[name];
            let sub_content = fetch_sub_config(url)
                .with_context(|| format!("cannot update subscription {name}"))?;
            write(&sub_path, sub_content)?;
            Ok::<(), anyhow::Error>(())
        })
        .for_each(|result| {
            if let Err(err) = result {
                eprintln!("Error: {err}");
            }
        });

    if need_reload {
        let name = &config.current_sub;
        config.reload_mihomo(name)?;
    }

    Ok(())
}

fn edit_override(name: Option<String>, editor: Option<String>) -> Result<()> {
    let override_path = if let Some(name) = &name {
        let config = MihuConfig::load()?;
        ensure!(config.subs.contains_key(name), "subscription \"{name}\" does not exist");
        get_sub_path(name, true)
    } else {
        dirs::config_dir().unwrap().join("mihu/global_override.yaml")
    };

    let editor = if let Some(editor) = editor {
        editor
    } else {
        let editor_env = std::env::var("EDITOR").context("EDITOR variable not set")?;
        which(editor_env)?.to_string_lossy().into_owned()
    };

    std::process::Command::new(editor).arg(override_path).status()?;

    let config = MihuConfig::load()?;
    if name.is_none() || name.as_ref() == Some(&config.current_sub) {
        config.reload_mihomo(&config.current_sub)?;
    }

    Ok(())
}

fn dashboard(edit: Option<String>) -> Result<()> {
    let mut config = MihuConfig::load()?;
    if let Some(url) = edit {
        ensure!(Url::parse(&url).is_ok(), "invalid URL");
        config.dashboard_url = url;
        config.save()
    } else {
        webbrowser::open(&config.dashboard_url).context("cannot open browser")
    }
}

fn print_info(verbose: bool, raw: bool, mihomo: bool) -> Result<()> {
    if raw {
        println!("{}", read_to_string(dirs::config_dir().unwrap().join("mihu/config.yaml"))?);
        return Ok(());
    }

    if mihomo {
        println!("{}", read_to_string(MihuConfig::load()?.mihomo_path)?);
        return Ok(());
    }

    let config = MihuConfig::load()?;

    if verbose {
        println!("Mihomo config file: {}", config.mihomo_path.display());
        println!("Mihomo external control endpoint: {}", config.external_control_url);
        println!("Dashboard website: {}", config.dashboard_url);
    }

    println!("Current subscription: {}", config.current_sub);
    println!("Default subscription: {}", config.default_sub);
    println!("All subscriptions:");

    for (key, value) in &config.subs {
        if verbose {
            println!("  {}: {}", key, value);
        } else {
            println!("  {}", key);
        }
    }

    Ok(())
}

fn init_app(
    mihomo_path: Option<PathBuf>,
    extctl_url: Option<String>,
    dashboard_url: Option<String>,
) -> Result<()> {
    let config_dir = dirs::config_dir().unwrap().join("mihu");
    if !config_dir.exists() {
        create_dir_all(&config_dir)?;
    }
    let sub_dir = config_dir.join("subscriptions");
    if !sub_dir.exists() {
        create_dir(sub_dir)?;
    }
    let ovrd_dir = config_dir.join("overrides");
    if !ovrd_dir.exists() {
        create_dir(ovrd_dir)?;
    }

    let config_path = config_dir.join("config.yaml");
    if !config_path.exists() {
        let mut config = MihuConfig::default();
        if let Some(path) = mihomo_path {
            config.mihomo_path = absolute(path)?;
        }
        if let Some(url) = extctl_url {
            ensure!(Url::parse(&url).is_ok(), "invalid URL");
            config.external_control_url = format!("{}/", url.trim_end_matches('/'));
        }
        if let Some(url) = dashboard_url {
            ensure!(Url::parse(&url).is_ok(), "invalid URL");
            config.dashboard_url = url;
        }
        config.save()?;
    }
    let global_override_path = config_dir.join("global_override.yaml");
    if !global_override_path.exists() {
        write(global_override_path, "{}")?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(command) = cli.command {
        match command {
            Command::Sub { name, url, switch, default } => add_sub(name, url, switch, default),
            Command::Remove { name, remove_override } => remove_sub(name, remove_override),
            Command::Switch { name, update, default } => switch_sub(name, update, default),
            Command::Update { names, all } => update_subs(names, all),
            Command::Edit { name, editor } => edit_override(name, editor),
            Command::Dash { edit } => dashboard(edit),
            Command::Info { verbose, raw, mihomo } => print_info(verbose, raw, mihomo),
            Command::Init { mihomo_config, extctl_url, dashboard_url } => {
                init_app(mihomo_config, extctl_url, dashboard_url)
            }
        }
    } else {
        update_subs(Vec::new(), false)
    }
}
