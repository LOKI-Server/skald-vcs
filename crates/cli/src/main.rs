use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use skald_vcs_client::{Merge, Remote, Repository, WorkspaceChange};
use skald_vcs_core::Settings;
use std::{path::PathBuf, time::Duration};

#[derive(Parser)]
#[command(version, about = "Skald")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create an offline working copy.
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Clone a repository into a new directory.
    Clone {
        url: String,
        destination: Option<PathBuf>,
        /// Download metadata first and only materialize explicitly included paths.
        #[arg(long)]
        sparse: bool,
        /// Include a root-relative file or directory. Repeat to include multiple paths.
        #[arg(long, value_name = "PATH")]
        include: Vec<String>,
    },
    /// Choose which paths are present in this working copy.
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    Status {
        #[arg(long)]
        refresh: bool,
    },
    Add {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    Unstage {
        #[arg(required = true)]
        paths: Vec<String>,
    },
    Commit {
        #[arg(short, long)]
        message: String,
        #[arg(long)]
        author: Option<String>,
    },
    /// List, create, or delete a branch.
    Branch {
        name: Option<String>,
        #[arg(long)]
        from: Option<String>,
        #[arg(short, long)]
        delete: bool,
        #[arg(long, requires = "delete")]
        force: bool,
    },
    Switch {
        branch: String,
    },
    Checkout {
        revision: String,
    },
    Restore {
        revision: String,
        #[arg(required = true)]
        paths: Vec<String>,
    },
    Log {
        revision: Option<String>,
        #[arg(short = 'n', long, default_value_t = 30)]
        limit: usize,
    },
    Diff {
        from: Option<String>,
        to: Option<String>,
        #[arg(long)]
        staged: bool,
    },
    Merge {
        revision: Option<String>,
        #[arg(long)]
        abort: bool,
        #[arg(short, long)]
        message: Option<String>,
        #[arg(long)]
        author: Option<String>,
    },
    /// List, create, or delete a tag.
    Tag {
        name: Option<String>,
        #[arg(long)]
        from: Option<String>,
        #[arg(short, long)]
        delete: bool,
    },
    #[command(subcommand)]
    Remote(RemoteCommand),
    Fetch {
        #[arg(default_value = "origin")]
        remote: String,
    },
    Pull {
        #[arg(default_value = "origin")]
        remote: String,
        branch: Option<String>,
        #[arg(long)]
        author: Option<String>,
    },
    Push {
        #[arg(default_value = "origin")]
        remote: String,
        name: Option<String>,
        #[arg(long)]
        tag: bool,
        #[arg(short, long)]
        delete: bool,
    },
    /// List asset locks for a remote.
    Locks {
        #[arg(default_value = "origin")]
        remote: String,
    },
    /// Acquire or renew an advisory asset lock.
    Lock {
        path: String,
        #[arg(long, default_value = "origin")]
        remote: String,
        #[arg(long, default_value_t = 3600)]
        seconds: u32,
    },
    Unlock {
        path: String,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Configure chunk sizes in bytes and the Zstandard level.
    Config {
        #[arg(long)]
        min: usize,
        #[arg(long)]
        average: usize,
        #[arg(long)]
        max: usize,
        #[arg(long, default_value_t = 3)]
        compression: i32,
    },
    Verify,
    Gc {
        #[arg(long, default_value_t = 24)]
        grace_hours: u64,
    },
}

#[derive(Subcommand)]
enum RemoteCommand {
    List,
    Set { name: String, url: String },
    Remove { name: String },
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    /// Show the saved include and exclude rules.
    List,
    /// List tracked paths, sizes, and whether they are included.
    Files { revision: Option<String> },
    /// Replace the selection. Use . to select everything or no paths for metadata only.
    Set {
        paths: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Include files or directories, downloading their current contents as needed.
    Add {
        #[arg(required = true)]
        paths: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove files or directories from this working copy.
    Remove {
        #[arg(required = true)]
        paths: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Reclaim cached chunks that origin still holds and local work doesn't need.
    Prune,
}

fn workspace(repo: &Repository, command: WorkspaceCommand) -> Result<()> {
    let (change, dry_run) = match command {
        WorkspaceCommand::List => {
            for (path, include) in repo.workspace_selection()?.rules() {
                println!("{} {path}", if *include { "include" } else { "exclude" });
            }
            return Ok(());
        }
        WorkspaceCommand::Files { revision } => {
            for file in repo.workspace_files(revision.as_deref())? {
                let selection = if file.included { "+" } else { "-" };
                println!("{selection} {:>12} {}", file.size, file.path);
            }
            return Ok(());
        }
        WorkspaceCommand::Set { paths, dry_run } => (WorkspaceChange::Set(paths), dry_run),
        WorkspaceCommand::Add { paths, dry_run } => (WorkspaceChange::Add(paths), dry_run),
        WorkspaceCommand::Remove { paths, dry_run } => (WorkspaceChange::Remove(paths), dry_run),
        WorkspaceCommand::Prune => {
            println!("Removed {} cached chunks", repo.prune_workspace(&token())?);
            return Ok(());
        }
    };
    let plan = repo.change_workspace(&change, dry_run, Some(&token()))?;
    for path in plan.add {
        println!("+ {path}");
    }
    for path in plan.remove {
        println!("- {path}");
    }
    Ok(())
}

fn main() {
    if let Err(error) = run(Cli::parse().command) {
        eprintln!("skald: {error:#}");
        std::process::exit(1);
    }
}

fn token() -> String {
    std::env::var("SKALD_TOKEN").unwrap_or_default()
}

fn author(value: Option<String>) -> Result<String> {
    value
        .or_else(|| std::env::var("SKALD_AUTHOR").ok())
        .filter(|s| !s.trim().is_empty())
        .context("set SKALD_AUTHOR or pass --author")
}

fn report_merge(result: Merge) -> Result<()> {
    if let Some(id) = result.revision {
        println!("{id}");
    }
    for path in &result.conflicts {
        eprintln!("Conflict {path}");
    }
    ensure!(
        result.conflicts.is_empty(),
        "resolve the listed paths, stage them and commit, or abort the merge"
    );
    Ok(())
}

fn configured(repo: &Repository, name: &str) -> Result<Remote> {
    Remote::new(
        repo.remotes()?.get(name).context("unknown remote")?,
        token(),
    )
}

fn run(command: Command) -> Result<()> {
    let repo = match command {
        Command::Init { path } => {
            let repo = Repository::init(&path, Settings::default())?;
            println!("Initialized {}", repo.root().display());
            return Ok(());
        }
        Command::Clone {
            url,
            destination,
            sparse,
            include,
        } => {
            let destination = destination.unwrap_or_else(|| {
                PathBuf::from(
                    url.trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or("working-copy"),
                )
            });
            let repo = if sparse || !include.is_empty() {
                Repository::clone_sparse(&destination, &url, &token(), &include)?
            } else {
                Repository::clone_remote(&destination, &url, &token())?
            };
            println!("Cloned {}", repo.root().display());
            return Ok(());
        }
        _ => Repository::discover(std::env::current_dir()?)?,
    };
    match command {
        Command::Init { .. } | Command::Clone { .. } => unreachable!(),
        Command::Workspace(action) => workspace(&repo, action)?,
        Command::Status { refresh } => {
            let status = repo.status_with_refresh(refresh)?;
            println!(
                "{}",
                status.branch.unwrap_or_else(|| format!(
                    "Detached {}",
                    status.revision.map(|r| r.to_string()).unwrap_or_default()
                ))
            );
            let clean = status.staged.is_empty()
                && status.unstaged.is_empty()
                && status.conflicts.is_empty();
            for (path, kind) in status.staged {
                println!("staged   {kind:8} {path}");
            }
            for (path, kind) in status.unstaged {
                println!("working  {kind:8} {path}");
            }
            for path in status.conflicts {
                println!("conflict {path}");
            }
            if clean {
                println!("Working copy is clean.");
            }
        }
        Command::Add { paths } => repo.stage_paths(&paths)?,
        Command::Unstage { paths } => repo.unstage(&paths)?,
        Command::Commit {
            author: who,
            message,
        } => println!("{}", repo.commit(&author(who)?, &message)?),
        Command::Branch {
            name,
            from,
            delete,
            force,
        } => {
            if let Some(name) = name {
                if delete {
                    repo.delete_branch(&name, force)?;
                } else {
                    repo.branch(&name, from.as_deref())?;
                }
            } else {
                ensure!(!delete && from.is_none(), "specify a branch name");
                for (name, _) in repo.branches()? {
                    println!("{name}");
                }
            }
        }
        Command::Switch { branch } => repo.checkout_with_token(&branch, false, Some(&token()))?,
        Command::Checkout { revision } => {
            repo.checkout_with_token(&revision, true, Some(&token()))?
        }
        Command::Restore { revision, paths } => {
            repo.restore_with_token(&revision, &paths, Some(&token()))?
        }
        Command::Log { revision, limit } => {
            for (id, commit) in repo.history(revision.as_deref(), limit)? {
                println!(
                    "{id}  {}  {}\n{}\n",
                    commit.author, commit.timestamp, commit.message
                );
            }
        }
        Command::Diff { from, to, staged } => {
            for change in repo.diff(from.as_deref(), to.as_deref(), staged)? {
                match change.text {
                    Some(text) if !text.is_empty() => print!("{text}"),
                    _ => println!("{} {}", change.change, change.path),
                }
            }
        }
        Command::Merge {
            revision,
            abort,
            message,
            author: who,
        } => {
            if abort {
                ensure!(revision.is_none(), "omit revision when aborting");
                repo.abort_merge()?;
            } else {
                let revision = revision.context("specify a revision to merge")?;
                report_merge(repo.merge_with_token(
                    &revision,
                    &author(who)?,
                    &message.unwrap_or_else(|| format!("Merge {revision}")),
                    Some(&token()),
                )?)?;
            }
        }
        Command::Tag { name, from, delete } => {
            if let Some(name) = name {
                repo.tag(&name, from.as_deref(), delete)?;
            } else {
                ensure!(!delete && from.is_none(), "specify a tag name");
                for (name, id) in repo.tags()? {
                    println!("{name} {id}");
                }
            }
        }
        Command::Remote(RemoteCommand::List) => {
            for (name, url) in repo.remotes()? {
                println!("{name} {url}");
            }
        }
        Command::Remote(RemoteCommand::Set { name, url }) => repo.remote(&name, Some(&url))?,
        Command::Remote(RemoteCommand::Remove { name }) => repo.remote(&name, None)?,
        Command::Fetch { remote } => {
            let refs = repo.fetch(&remote, &token())?;
            println!(
                "Fetched {} branches and {} tags",
                refs.branches.len(),
                refs.tags.len()
            );
        }
        Command::Pull {
            remote,
            branch,
            author: who,
        } => report_merge(repo.pull(&remote, &token(), branch.as_deref(), &author(who)?)?)?,
        Command::Push {
            remote,
            name,
            tag,
            delete,
        } => repo.push(&remote, &token(), name.as_deref(), tag, delete)?,
        Command::Locks { remote } => println!(
            "{}",
            serde_json::to_string_pretty(&configured(&repo, &remote)?.locks()?)?
        ),
        Command::Lock {
            path,
            remote,
            seconds,
        } => {
            let lock = configured(&repo, &remote)?.lock(&path, Some(seconds))?;
            println!("{}", serde_json::to_string(&lock)?);
        }
        Command::Unlock { path, remote } => {
            configured(&repo, &remote)?.lock(&path, None)?;
        }
        Command::Config {
            min,
            average,
            max,
            compression,
        } => repo.configure_storage(Settings {
            min,
            average,
            max,
            compression,
        })?,
        Command::Verify => {
            repo.verify()?;
            println!("Repository verified.");
        }
        Command::Gc { grace_hours } => {
            println!(
                "Removed {} objects",
                repo.gc(Duration::from_secs(grace_hours.saturating_mul(3600)))?
            );
        }
    }
    Ok(())
}
