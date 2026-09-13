use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use keel_sdk::{canonical_json, Client, ContentId, Identity, JobSpec};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "keel",
    about = "Keel operator CLI. Pure functions locally; everything else is the node HTTP API so other frontends can speak the same paths."
)]
struct Cli {
    /// Node HTTP API (dashboard is the same origin at GET /)
    #[arg(long, env = "KEEL_API", global = true, default_value = "http://127.0.0.1:7420")]
    api: String,
    /// Opt-in client FilterList CID. Blob GET on the node still succeeds; this CLI will not print denied CIDs.
    #[arg(long, env = "KEEL_FILTER_CID", global = true)]
    filter_cid: Option<String>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Protocol version (local, no node required)
    Version,
    /// SHA-256 a file as sha256:<hex> (local)
    Cid { path: PathBuf },
    /// Canonical-JSON hash a JSON file (local)
    Canonical { path: PathBuf },
    /// Node operator identity (local key file, not HTTP)
    Identity {
        #[command(subcommand)]
        command: IdentityCmd,
    },
    /// Run a local node (HTTP API + operator dashboard)
    Node {
        #[command(subcommand)]
        command: NodeCmd,
    },
    /// GET /v0/status
    Status,
    /// Print GET /v0 (machine-readable route catalog for BYO frontends)
    Api,
    Account {
        #[command(subcommand)]
        command: AccountCmd,
    },
    Blob {
        #[command(subcommand)]
        command: BlobCmd,
    },
    Artifact {
        #[command(subcommand)]
        command: ArtifactCmd,
    },
    Index {
        #[command(subcommand)]
        command: IndexCmd,
    },
    Seeder {
        #[command(subcommand)]
        command: SeederCmd,
    },
    Peer {
        #[command(subcommand)]
        command: PeerCmd,
    },
    Credit {
        #[command(subcommand)]
        command: CreditCmd,
    },
    Pay {
        #[command(subcommand)]
        command: PayCmd,
    },
    Job {
        #[command(subcommand)]
        command: JobCmd,
    },
}

#[derive(Subcommand)]
enum IdentityCmd {
    Show {
        #[arg(long, env = "KEEL_DATA_DIR")]
        data_dir: Option<PathBuf>,
    },
    Sign {
        path: PathBuf,
        #[arg(long, env = "KEEL_DATA_DIR")]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum NodeCmd {
    /// Bind HTTP: JSON under /v0, HTML dashboard at /
    Serve {
        #[arg(long, default_value = "127.0.0.1:7420")]
        bind: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// public = gossip this address; invite = only reachable with a signed invite (default)
        #[arg(long, env = "KEEL_PEER_VISIBILITY", default_value = "invite")]
        visibility: String,
        /// Public nodes to pull peer ads from (repeatable). Also KEEL_BOOTSTRAP=url,url
        #[arg(long)]
        bootstrap: Vec<String>,
    },
}

#[derive(Subcommand)]
enum AccountCmd {
    New,
    List,
    Show { hex: String },
}

#[derive(Subcommand)]
enum BlobCmd {
    Put { path: PathBuf },
    Get {
        cid: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    List,
}

#[derive(Subcommand)]
enum ArtifactCmd {
    Announce { path: PathBuf },
    Show { cid: String },
}

#[derive(Subcommand)]
enum IndexCmd {
    Publish { path: PathBuf },
    Head { publisher: String },
}

#[derive(Subcommand)]
enum SeederCmd {
    /// GET /v0/seeders or /v0/seeders/{cid} — hosts this node has been told about
    List { cid: Option<String> },
    /// Tell this node another host has bytes for a CID
    Announce {
        cid: String,
        /// e.g. /ip4/203.0.113.9/tcp/7420/http
        #[arg(long)]
        multiaddr: String,
        #[arg(long)]
        expires_at: Option<u64>,
    },
}

#[derive(Subcommand)]
enum PeerCmd {
    /// Public peer ads this node will gossip (GET /v0/peers)
    List,
    /// Operator address book including invite-only peers
    Known,
    /// Pull public ads from bootstrap / known public peers
    Sync {
        #[arg(long)]
        url: Vec<String>,
    },
    /// Write a signed invite (does not list you publicly)
    Invite {
        #[arg(long)]
        once: bool,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Accept an invite file; redeem so the issuer also learns you
    Accept { path: PathBuf },
}

#[derive(Subcommand)]
enum CreditCmd {
    Mint {
        account: String,
        amount: i64,
        #[arg(long)]
        rail_ref: String,
        #[arg(long, default_value = "mock")]
        rail: String,
    },
    Show { account: String },
    Movements,
}

#[derive(Subcommand)]
enum PayCmd {
    Intent {
        account: String,
        millicredits: i64,
        #[arg(long, default_value_t = 1)]
        sats: u64,
    },
    Settle {
        payment_hash: String,
        preimage: String,
    },
}

#[derive(Subcommand)]
enum JobCmd {
    Submit {
        spec: PathBuf,
        /// Submit to this runner's /v0 instead of --api
        #[arg(long)]
        runner: Option<String>,
    },
    Accept { id: String },
    List,
    Show { id: String },
    Run {
        id: String,
        #[arg(long)]
        runner: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Version => {
            println!("keel {}", keel_sdk::KEEL_VERSION);
        }
        Commands::Cid { path } => {
            let bytes = std::fs::read(&path).with_context(|| path.display().to_string())?;
            println!("{}", ContentId::of_bytes(&bytes));
        }
        Commands::Canonical { path } => {
            let bytes = std::fs::read(&path).with_context(|| path.display().to_string())?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            let canon = canonical_json(&value).map_err(|e| anyhow::anyhow!(e))?;
            println!("{}", ContentId::of_bytes(&canon));
        }
        Commands::Identity { command } => match command {
            IdentityCmd::Show { data_dir } => {
                let dir = data_dir.unwrap_or_else(default_data_dir);
                let id = Identity::load_or_create(&dir.join("identity.key"))
                    .map_err(|e| anyhow::anyhow!(e))?;
                print_json(&serde_json::json!({
                    "pubkey": id.pubkey().to_hex(),
                    "path": dir.join("identity.key").display().to_string(),
                }))?;
            }
            IdentityCmd::Sign { path, data_dir } => {
                let dir = data_dir.unwrap_or_else(default_data_dir);
                let id = Identity::load_or_create(&dir.join("identity.key"))
                    .map_err(|e| anyhow::anyhow!(e))?;
                let bytes = std::fs::read(&path)?;
                let sig = id.sign(&bytes);
                print_json(&serde_json::json!({
                    "pubkey": id.pubkey().to_hex(),
                    "signature": hex::encode(sig.0),
                }))?;
            }
        },
        Commands::Node {
            command:
                NodeCmd::Serve {
                    bind,
                    data_dir,
                    visibility,
                    bootstrap,
                },
        } => {
            let addr: SocketAddr = bind.parse().context("bind")?;
            let dir = data_dir.unwrap_or_else(default_data_dir);
            let vis = keel_sdk::PeerVisibility::parse(&visibility).map_err(|e| anyhow::anyhow!(e))?;
            let mut boot = bootstrap;
            if boot.is_empty() {
                if let Ok(s) = std::env::var("KEEL_BOOTSTRAP") {
                    boot = s
                        .split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect();
                }
            }
            eprintln!("keel node  http://{addr}/      dashboard");
            eprintln!("           http://{addr}/v0   API catalog");
            eprintln!("data-dir   {}", dir.display());
            eprintln!("visibility {}", vis.as_str());
            keel_node::serve_with(addr, dir, vis, boot).await?;
        }
        Commands::Status => {
            print_json(&Client::new(&cli.api).status().await.map_err(aj)?)?;
        }
        Commands::Api => {
            print_json(&Client::new(&cli.api).api_index().await.map_err(aj)?)?;
        }
        Commands::Account { command } => {
            let c = Client::new(&cli.api);
            match command {
                AccountCmd::New => print_json(&c.new_account().await.map_err(aj)?)?,
                AccountCmd::List => print_json(&c.list_accounts().await.map_err(aj)?)?,
                AccountCmd::Show { hex } => {
                    print_json(&c.get_account(&hex).await.map_err(aj)?)?
                }
            }
        }
        Commands::Blob { command } => {
            let c = Client::new(&cli.api);
            match command {
                BlobCmd::Put { path } => {
                    let bytes = std::fs::read(&path)?;
                    print_json(&c.put_blob(&bytes).await.map_err(aj)?)?;
                }
                BlobCmd::Get { cid, out } => {
                    let cid = ContentId::from_hex(&cid).map_err(|e| anyhow::anyhow!(e))?;
                    if let Some(filter) = &cli.filter_cid {
                        refuse_if_denied(&c, filter, &cid).await?;
                    }
                    let bytes = c.get_blob(&cid).await.map_err(aj)?;
                    if let Some(out) = out {
                        std::fs::write(&out, &bytes)?;
                    } else {
                        std::io::Write::write_all(&mut std::io::stdout(), &bytes)?;
                    }
                }
                BlobCmd::List => print_json(&c.list_blobs().await.map_err(aj)?)?,
            }
        }
        Commands::Artifact { command } => {
            let c = Client::new(&cli.api);
            match command {
                ArtifactCmd::Announce { path } => {
                    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
                    print_json(&c.put_artifact(&v).await.map_err(aj)?)?;
                }
                ArtifactCmd::Show { cid } => {
                    let cid = ContentId::from_hex(&cid).map_err(|e| anyhow::anyhow!(e))?;
                    print_json(&c.get_artifact(&cid).await.map_err(aj)?)?;
                }
            }
        }
        Commands::Index { command } => {
            let c = Client::new(&cli.api);
            match command {
                IndexCmd::Publish { path } => {
                    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
                    print_json(&c.publish_index(&v).await.map_err(aj)?)?;
                }
                IndexCmd::Head { publisher } => {
                    print_json(&c.index_head(&publisher).await.map_err(aj)?)?;
                }
            }
        }
        Commands::Seeder { command } => {
            let c = Client::new(&cli.api);
            match command {
                SeederCmd::List { cid } => {
                    let cid = cid
                        .as_deref()
                        .map(ContentId::from_hex)
                        .transpose()
                        .map_err(|e| anyhow::anyhow!(e))?;
                    print_json(&c.list_seeders(cid.as_ref()).await.map_err(aj)?)?;
                }
                SeederCmd::Announce {
                    cid,
                    multiaddr,
                    expires_at,
                } => {
                    let cid = ContentId::from_hex(&cid).map_err(|e| anyhow::anyhow!(e))?;
                    let expires_at = expires_at.unwrap_or_else(|| {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() + 86400)
                            .unwrap_or(0)
                    });
                    print_json(
                        &c.put_seeder(&serde_json::json!({
                            "file_cid": cid.to_string(),
                            "multiaddrs": [multiaddr],
                            "expires_at": expires_at
                        }))
                        .await
                        .map_err(aj)?,
                    )?;
                }
            }
        }
        Commands::Peer { command } => {
            let c = Client::new(&cli.api);
            match command {
                PeerCmd::List => print_json(&c.list_public_peers().await.map_err(aj)?)?,
                PeerCmd::Known => print_json(&c.list_known_peers().await.map_err(aj)?)?,
                PeerCmd::Sync { url } => print_json(&c.sync_peers(&url).await.map_err(aj)?)?,
                PeerCmd::Invite { once, out } => {
                    let v = c.create_invite(once).await.map_err(aj)?;
                    if let Some(out) = out {
                        std::fs::write(&out, serde_json::to_vec_pretty(&v)?)?;
                    } else {
                        print_json(&v)?;
                    }
                }
                PeerCmd::Accept { path } => {
                    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
                    print_json(&c.accept_invite(&v).await.map_err(aj)?)?;
                }
            }
        }
        Commands::Credit { command } => {
            let c = Client::new(&cli.api);
            match command {
                CreditCmd::Mint {
                    account,
                    amount,
                    rail_ref,
                    rail,
                } => print_json(
                    &c.mint_on_rail(&account, amount, &rail_ref, &rail)
                        .await
                        .map_err(aj)?,
                )?,
                CreditCmd::Show { account } => {
                    print_json(&c.get_account(&account).await.map_err(aj)?)?
                }
                CreditCmd::Movements => print_json(&c.list_movements().await.map_err(aj)?)?,
            }
        }
        Commands::Pay { command } => {
            let c = Client::new(&cli.api);
            match command {
                PayCmd::Intent {
                    account,
                    millicredits,
                    sats,
                } => print_json(&c.pay_intent(&account, millicredits, sats).await.map_err(aj)?)?,
                PayCmd::Settle {
                    payment_hash,
                    preimage,
                } => print_json(&c.pay_settle(&payment_hash, &preimage).await.map_err(aj)?)?,
            }
        }
        Commands::Job { command } => match command {
            JobCmd::Submit { spec, runner } => {
                let api = runner.as_deref().unwrap_or(&cli.api);
                let c = Client::new(api);
                let spec: JobSpec = serde_json::from_slice(&std::fs::read(&spec)?)?;
                print_json(&c.submit_job(&spec).await.map_err(aj)?)?;
            }
            JobCmd::Accept { id } => {
                let c = Client::new(&cli.api);
                let id = ContentId::from_hex(&id).map_err(|e| anyhow::anyhow!(e))?;
                print_json(&c.accept_job(&id).await.map_err(aj)?)?;
            }
            JobCmd::List => print_json(&Client::new(&cli.api).list_jobs().await.map_err(aj)?)?,
            JobCmd::Show { id } => {
                let id = ContentId::from_hex(&id).map_err(|e| anyhow::anyhow!(e))?;
                print_json(&Client::new(&cli.api).get_job(&id).await.map_err(aj)?)?;
            }
            JobCmd::Run { id, runner } => {
                let api = runner.as_deref().unwrap_or(&cli.api);
                let c = Client::new(api);
                let id = ContentId::from_hex(&id).map_err(|e| anyhow::anyhow!(e))?;
                print_json(&c.run_job(&id).await.map_err(aj)?)?;
            }
        },
    }
    Ok(())
}

async fn refuse_if_denied(c: &Client, filter_cid: &str, blob: &ContentId) -> Result<()> {
    let cid = ContentId::from_hex(filter_cid).map_err(|e| anyhow::anyhow!(e))?;
    let f = c.get_filter(&cid).await.map_err(aj)?;
    let denied = f
        .pointer("/body/deny_cids")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let want = blob.to_string();
    let hex = blob.to_hex();
    if denied.iter().any(|d| {
        d.as_str() == Some(want.as_str()) || d.as_str() == Some(hex.as_str())
    }) {
        anyhow::bail!("denied by client FilterList {filter_cid} (node blob GET still succeeds)");
    }
    Ok(())
}

fn print_json(v: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn aj(e: keel_sdk::ClientError) -> anyhow::Error {
    anyhow::anyhow!(e)
}

fn default_data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("KEEL_DATA_DIR") {
        return PathBuf::from(p);
    }
    let mut p = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    p.push(".keel");
    p
}
