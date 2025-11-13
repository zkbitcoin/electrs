use bitcoin::p2p::Magic;
use bitcoin::Network;
use bitcoincore_rpc::Auth;
use dirs_next::home_dir;

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::str::FromStr;

use std::env::consts::{ARCH, OS};
use std::time::Duration;

pub const ELECTRS_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_SERVER_ADDRESS: [u8; 4] = [127, 0, 0, 1];

mod internal {
    #![allow(unused_imports)]
    include!(concat!(env!("OUT_DIR"), "/configure_me_config.rs"));
}

/// A simple error type representing invalid UTF-8 input.
pub struct InvalidUtf8(OsString);

impl fmt::Display for InvalidUtf8 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?} isn't a valid UTF-8 sequence", self.0)
    }
}

/// An error that might happen when resolving an address
pub enum AddressError {
    ResolvError { addr: String, err: std::io::Error },
    NoAddrError(String),
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AddressError::ResolvError { addr, err } => {
                write!(f, "Failed to resolve address {}: {}", addr, err)
            }
            AddressError::NoAddrError(addr) => write!(f, "No address found for {}", addr),
        }
    }
}

/// Newtype for addresses parsed from strings
#[derive(Deserialize)]
pub struct ResolvAddr(String);

impl ::configure_me::parse_arg::ParseArg for ResolvAddr {
    type Error = InvalidUtf8;

    fn parse_arg(arg: &OsStr) -> Result<Self, Self::Error> {
        Self::parse_owned_arg(arg.to_owned())
    }

    fn parse_owned_arg(arg: OsString) -> Result<Self, Self::Error> {
        arg.into_string().map_err(InvalidUtf8).map(ResolvAddr)
    }

    fn describe_type<W: fmt::Write>(mut writer: W) -> fmt::Result {
        write!(writer, "a network address (will be resolved if needed)")
    }
}

impl ResolvAddr {
    fn resolve(self) -> Result<SocketAddr, AddressError> {
        match self.0.to_socket_addrs() {
            Ok(mut iter) => iter.next().ok_or(AddressError::NoAddrError(self.0)),
            Err(err) => Err(AddressError::ResolvError { addr: self.0, err }),
        }
    }

    fn resolve_or_exit(self) -> SocketAddr {
        self.resolve().unwrap_or_else(|err| {
            eprintln!("Error: {}", err);
            std::process::exit(1)
        })
    }
}

/// Parsed, post-processed configuration
#[derive(Debug)]
pub struct Config {
    pub network: String,            // RAW STRING ("bitcoin", "pivx", etc)
    pub btc_network: Network,
    pub db_path: PathBuf,
    pub db_log_dir: Option<PathBuf>,
    pub db_parallelism: u8,
    pub daemon_auth: SensitiveAuth,
    pub daemon_rpc_addr: SocketAddr,
    pub daemon_p2p_addr: SocketAddr,
    pub electrum_rpc_addr: SocketAddr,
    pub monitoring_addr: SocketAddr,
    pub wait_duration: Duration,
    pub jsonrpc_timeout: Duration,
    pub index_batch_size: usize,
    pub index_lookup_limit: Option<usize>,
    pub reindex_last_blocks: usize,
    pub auto_reindex: bool,
    pub ignore_mempool: bool,
    pub sync_once: bool,
    pub skip_block_download_wait: bool,
    pub disable_electrum_rpc: bool,
    pub server_banner: String,
    pub signet_magic: Magic,
}

pub struct SensitiveAuth(pub Auth);

impl SensitiveAuth {
    pub(crate) fn get_auth(&self) -> Auth {
        self.0.clone()
    }
}

impl fmt::Debug for SensitiveAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Auth::UserPass(ref user, _) =>
                f.debug_tuple("UserPass").field(&user).field(&"<sensitive>").finish(),
            _ => write!(f, "{:?}", self.0),
        }
    }
}

/// Default daemon dir
fn default_daemon_dir() -> PathBuf {
    let mut home = home_dir().unwrap_or_else(|| {
        eprintln!("Error: unknown home directory");
        std::process::exit(1)
    });
    home.push(".bitcoin");
    home
}

/// Config file loading paths
fn default_config_files() -> Vec<OsString> {
    let mut files = vec![OsString::from("electrs.toml")];
    if let Some(mut path) = home_dir() {
        path.extend([".electrs", "config.toml"]);
        files.push(OsString::from(path));
    }
    files.push(OsString::from("/etc/electrs/config.toml"));
    files
}

impl Config {
    pub fn from_args() -> Config {
        use internal::prelude::ResultExt;

        let (mut config, _args) =
            internal::prelude::Config::including_optional_config_files(default_config_files())
                .unwrap_or_exit();

        // --------------------------------------------------------------
        // RAW NETWORK STRING
        // --------------------------------------------------------------
        let raw_network = config.network.clone();
        let is_pivx = raw_network == "pivx";

        // --------------------------------------------------------------
        // Convert into bitcoin::Network for Bitcoin-side logic
        // --------------------------------------------------------------
        let btc_network = if is_pivx {
            Network::Bitcoin // placeholder; PIVX overrides all ports/magic
        } else {
            Network::from_str(&raw_network)
                .unwrap_or_else(|_| {
                    eprintln!("Invalid network '{}'", raw_network);
                    std::process::exit(1)
                })
        };

        // --------------------------------------------------------------
        // Database subdirectories
        // --------------------------------------------------------------
        let db_subdir = if is_pivx {
            "pivx"
        } else {
            match btc_network {
                Network::Bitcoin => "bitcoin",
                Network::Testnet => "testnet",
                Network::Testnet4 => "testnet4",
                Network::Regtest => "regtest",
                Network::Signet => "signet",
            }
        };

        config.db_dir.push(db_subdir);

        // --------------------------------------------------------------
        // Default ports
        // --------------------------------------------------------------
        let default_daemon_rpc_port = if is_pivx {
            8050
        } else {
            match btc_network {
                Network::Bitcoin => 8332,
                Network::Testnet => 18332,
                Network::Testnet4 => 48332,
                Network::Regtest => 18443,
                Network::Signet => 38332,
            }
        };

        let default_daemon_p2p_port = if is_pivx {
            51472
        } else {
            match btc_network {
                Network::Bitcoin => 8333,
                Network::Testnet => 18333,
                Network::Testnet4 => 48333,
                Network::Regtest => 18444,
                Network::Signet => 38333,
            }
        };

        let default_electrum_port = if is_pivx {
            50001
        } else {
            match btc_network {
                Network::Bitcoin => 50001,
                Network::Testnet => 60001,
                Network::Testnet4 => 40001,
                Network::Regtest => 60401,
                Network::Signet => 60601,
            }
        };

        let default_monitoring_port = if is_pivx {
            4224
        } else {
            match btc_network {
                Network::Bitcoin => 4224,
                Network::Testnet => 14224,
                Network::Testnet4 => 44224,
                Network::Regtest => 24224,
                Network::Signet => 34224,
            }
        };

        // --------------------------------------------------------------
        // Magic Bytes
        // --------------------------------------------------------------
        let magic = if is_pivx {
            // PIVX MUST provide magic in config
            config
                .signet_magic
                .as_ref()
                .expect("PIVX mode requires 'signet_magic' in config")
                .parse()
                .unwrap_or_else(|e| {
                    eprintln!("Invalid PIVX magic: {}", e);
                    std::process::exit(1);
                })
        } else {
            match (btc_network, &config.signet_magic) {
                (Network::Signet, Some(hex)) => {
                    hex.parse().unwrap_or_else(|e| {
                        eprintln!("Invalid signet magic: {}", e);
                        std::process::exit(1)
                    })
                }
                (net, None) => net.magic(),
                (_, Some(_)) => {
                    eprintln!("signet_magic only allowed for signet (or pivx)");
                    std::process::exit(1);
                }
            }
        };

        // --------------------------------------------------------------
        // Address resolution
        // --------------------------------------------------------------

        let daemon_rpc_addr: SocketAddr = config.daemon_rpc_addr.map_or(
            (DEFAULT_SERVER_ADDRESS, default_daemon_rpc_port).into(),
            ResolvAddr::resolve_or_exit,
        );

        let daemon_p2p_addr: SocketAddr = config.daemon_p2p_addr.map_or(
            (DEFAULT_SERVER_ADDRESS, default_daemon_p2p_port).into(),
            ResolvAddr::resolve_or_exit,
        );

        let electrum_rpc_addr: SocketAddr = config.electrum_rpc_addr.map_or(
            (DEFAULT_SERVER_ADDRESS, default_electrum_port).into(),
            ResolvAddr::resolve_or_exit,
        );

        #[cfg(not(feature = "metrics"))]
        {
            if config.monitoring_addr.is_some() {
                eprintln!("Error: enable \"metrics\" feature to specify monitoring_addr");
                std::process::exit(1);
            }
        }

        let monitoring_addr: SocketAddr = config.monitoring_addr.map_or(
            (DEFAULT_SERVER_ADDRESS, default_monitoring_port).into(),
            ResolvAddr::resolve_or_exit,
        );

        // --------------------------------------------------------------
        // Daemon directory
        // --------------------------------------------------------------
        match btc_network {
            Network::Bitcoin => {
                if is_pivx {
                    // stays as /home/.../.pivx from TOML
                }
            }
            Network::Testnet => config.daemon_dir.push("testnet3"),
            Network::Testnet4 => config.daemon_dir.push("testnet4"),
            Network::Regtest => config.daemon_dir.push("regtest"),
            Network::Signet => config.daemon_dir.push("signet"),
        }

        // --------------------------------------------------------------
        // Deprecated options check
        // --------------------------------------------------------------
        let mut deprecated = false;

        if config.timestamp {
            eprintln!("Error: `timestamp` is deprecated");
            deprecated = true;
        }

        if config.verbose > 0 {
            eprintln!("Error: please use `log_filters` for verbosity");
            deprecated = true;
        }

        if deprecated {
            std::process::exit(1);
        }

        // --------------------------------------------------------------
        // Authentication handling
        // --------------------------------------------------------------
        let daemon_dir = &config.daemon_dir;
        let daemon_auth = SensitiveAuth(match (config.auth, config.cookie_file) {
            (None, None) => Auth::CookieFile(daemon_dir.join(".cookie")),
            (None, Some(path)) => Auth::CookieFile(path),
            (Some(auth), None) => {
                let parts: Vec<&str> = auth.splitn(2, ':').collect();
                if parts.len() != 2 {
                    eprintln!("Error: invalid auth format");
                    std::process::exit(1);
                }
                Auth::UserPass(parts[0].to_owned(), parts[1].to_owned())
            }
            (Some(_), Some(_)) => {
                eprintln!("Error: both auth and cookie_file set");
                std::process::exit(1);
            }
        });

        let log_filters = config.log_filters;

        let index_lookup_limit =
            if config.index_lookup_limit == 0 { None } else { Some(config.index_lookup_limit) };

        if config.jsonrpc_timeout_secs <= config.wait_duration_secs {
            eprintln!(
                "jsonrpc_timeout_secs ({}) must be > wait_duration_secs ({})",
                config.jsonrpc_timeout_secs, config.wait_duration_secs
            );
            std::process::exit(1);
        }

        if config.version {
            println!("v{}", ELECTRS_VERSION);
            std::process::exit(0);
        }

        // --------------------------------------------------------------
        // FINAL CONFIG STRUCT
        // --------------------------------------------------------------
        let final_config = Config {
            network: raw_network,
            btc_network,
            db_path: config.db_dir,
            db_log_dir: config.db_log_dir,
            db_parallelism: config.db_parallelism,
            daemon_auth,
            daemon_rpc_addr,
            daemon_p2p_addr,
            electrum_rpc_addr,
            monitoring_addr,
            wait_duration: Duration::from_secs(config.wait_duration_secs),
            jsonrpc_timeout: Duration::from_secs(config.jsonrpc_timeout_secs),
            index_batch_size: config.index_batch_size,
            index_lookup_limit,
            reindex_last_blocks: config.reindex_last_blocks,
            auto_reindex: config.auto_reindex,
            ignore_mempool: config.ignore_mempool,
            sync_once: config.sync_once,
            skip_block_download_wait: config.skip_block_download_wait,
            disable_electrum_rpc: config.disable_electrum_rpc,
            server_banner: config.server_banner,
            signet_magic: magic,
        };

        eprintln!(
            "Starting electrs {} on {} {} with {:?}",
            ELECTRS_VERSION, ARCH, OS, final_config
        );

        let mut builder = env_logger::Builder::from_default_env();
        builder.default_format().format_timestamp_millis();
        if let Some(log_filters) = &log_filters {
            builder.parse_filters(log_filters);
        }
        builder.init();

        final_config
    }
}

#[cfg(test)]
mod tests {
    use super::{Auth, SensitiveAuth};
    use std::path::Path;

    #[test]
    fn test_auth_debug() {
        let auth = Auth::None;
        assert_eq!(format!("{:?}", SensitiveAuth(auth)), "None");

        let auth = Auth::CookieFile(Path::new("/foo/bar/.cookie").to_path_buf());
        assert_eq!(
            format!("{:?}", SensitiveAuth(auth)),
            "CookieFile(\"/foo/bar/.cookie\")"
        );

        let auth = Auth::UserPass("user".to_owned(), "pass".to_owned());
        assert_eq!(
            format!("{:?}", SensitiveAuth(auth)),
            "UserPass(\"user\", \"<sensitive>\")"
        );
    }
}
