//! `vexus-cli`: the Vexus engine from a terminal.
//!
//! The same safety model as the desktop app: scans are read-only, `clean` only shows a plan
//! unless you pass `--yes`, and undoable removals go to the Recovery store.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use vexus_core::clean::CleanOptions;
use vexus_core::env::Env;
use vexus_core::model::{CategoryStatus, DeletionPolicy, Risk, Strategy};
use vexus_core::paths::{DataDirs, DataMode};
use vexus_core::progress::{CancelToken, NullSink, ScanEvent};
use vexus_core::scan::{Scan, ScanRequest, Selection};
use vexus_core::settings::Settings;
use vexus_core::util::{human_bytes, now_unix, stamp};
use vexus_core::{Engine, system};

#[derive(Parser)]
#[command(
    name = "vexus-cli",
    version,
    about = "Scan and clean Windows with the Vexus safety engine"
)]
struct Cli {
    /// Work on a fake Windows profile in DIR instead of this machine (see `sandbox create`).
    #[arg(long, global = true, value_name = "DIR")]
    sandbox: Option<PathBuf>,

    /// Log every path the engine touches.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the cleaning categories and how each one is treated
    Rules,
    /// Show what could be cleaned. Changes nothing.
    Scan(ScanArgs),
    /// Clean. Without --yes this only shows the plan.
    Clean(CleanArgs),
    /// Inspect, restore or free the Recovery store (undo)
    #[command(subcommand)]
    Recovery(RecoveryCmd),
    /// Create a fake Windows profile to try Vexus safely
    #[command(subcommand)]
    Sandbox(SandboxCmd),
}

#[derive(Args)]
struct ScanArgs {
    /// Only these categories (repeatable). Default: all.
    #[arg(short, long = "category", value_name = "ID")]
    categories: Vec<String>,
    /// Override the age limit (hours) for categories that allow it.
    #[arg(long, value_name = "HOURS")]
    min_age_hours: Option<u32>,
    /// Print the full result as JSON.
    #[arg(long)]
    json: bool,
    /// List the N largest items of each category.
    #[arg(long, value_name = "N", default_value_t = 0)]
    top: usize,
}

#[derive(Args)]
struct CleanArgs {
    /// Categories to clean (repeatable). Default: the recommended ones that were found.
    #[arg(short, long = "category", value_name = "ID")]
    categories: Vec<String>,
    #[arg(long, value_name = "HOURS")]
    min_age_hours: Option<u32>,
    /// Keep a recoverable copy of everything removed.
    #[arg(long, conflicts_with = "permanent")]
    undo: bool,
    /// Delete everything immediately, with no recoverable copy.
    #[arg(long)]
    permanent: bool,
    /// Actually clean. Without this, only the plan is shown.
    #[arg(short, long)]
    yes: bool,
}

#[derive(Subcommand)]
enum RecoveryCmd {
    /// List recoverable batches
    List,
    /// Put a batch back where it came from
    Restore { id: String },
    /// Permanently free a batch (or all expired ones with --expired)
    Purge {
        id: Option<String>,
        #[arg(long)]
        expired: bool,
    },
}

#[derive(Subcommand)]
enum SandboxCmd {
    /// Fill DIR with a realistic fake profile (junk to clean, files that must be kept)
    Create { dir: PathBuf },
}

struct Ctx {
    engine: Engine,
    data: DataDirs,
    settings: Settings,
}

fn open(cli: &Cli) -> Result<Ctx, Box<dyn std::error::Error>> {
    let (env, mode) = match &cli.sandbox {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            let root = vexus_core::fsops::canonical_dir(dir)?;
            (Env::sandbox(&root), DataMode::Custom(root.join(".vexus")))
        }
        None => {
            let exe_dir =
                std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
            (Env::detect(), DataMode::detect(exe_dir.as_deref()))
        }
    };
    let data = DataDirs::resolve(&mode, &env);
    data.ensure()?;
    let settings = Settings::load(&data.state);
    let engine = Engine::new(env, &data, system::is_elevated());
    Ok(Ctx { engine, data, settings })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(if cli.verbose {
            "debug"
        } else {
            "warn"
        }))
        .with_writer(std::io::stderr)
        .without_time()
        .init();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    if let Command::Sandbox(SandboxCmd::Create { dir }) = &cli.command {
        std::fs::create_dir_all(dir)?;
        let root = vexus_core::fsops::canonical_dir(dir)?;
        let summary = vexus_core::sandbox::populate(&Env::sandbox(&root))?;
        println!(
            "Created a sandbox profile in {}\n  {} files, {} logical size, {} of them protected.\n\nTry it:\n  vexus-cli --sandbox \"{}\" scan --top 3\n  vexus-cli --sandbox \"{}\" clean --yes",
            root.display(),
            summary.files,
            human_bytes(summary.bytes),
            summary.protected_files,
            root.display(),
            root.display()
        );
        return Ok(());
    }

    let ctx = open(cli)?;
    match &cli.command {
        Command::Rules => rules(&ctx),
        Command::Scan(args) => scan_cmd(&ctx, args)?,
        Command::Clean(args) => clean_cmd(&ctx, args)?,
        Command::Recovery(cmd) => recovery_cmd(&ctx, cmd)?,
        Command::Sandbox(_) => unreachable!("handled above"),
    }
    Ok(())
}

fn risk_label(r: Risk) -> &'static str {
    match r {
        Risk::Safe => "safe",
        Risk::Review => "review",
        Risk::Caution => "caution",
    }
}

fn rules(ctx: &Ctx) {
    println!("{:<30} {:<8} {:<10} {:<9} NAME", "ID", "RISK", "REMOVAL", "DEFAULT");
    for r in ctx.engine.rules() {
        println!(
            "{:<30} {:<8} {:<10} {:<9} {}{}",
            r.id,
            risk_label(r.risk),
            if r.strategy == Strategy::Undoable { "undoable" } else { "permanent" },
            if r.default_selected { "yes" } else { "no" },
            r.name,
            if r.needs_admin { "  (needs administrator)" } else { "" }
        );
    }
}

fn do_scan(
    ctx: &Ctx,
    categories: &[String],
    min_age: Option<u32>,
) -> Result<Scan, Box<dyn std::error::Error>> {
    let req = ScanRequest {
        categories: (!categories.is_empty()).then(|| categories.to_vec()),
        age_override_hours: min_age.or(Some(ctx.settings.temp_min_age_hours)),
        ..ScanRequest::default()
    };
    let tty = std::io::stderr().is_terminal();
    let sink = move |e: ScanEvent| {
        if let (true, ScanEvent::Progress(p)) = (tty, &e) {
            eprint!(
                "\r  scanning… {} entries · {} found   ",
                p.entries,
                human_bytes(p.found_bytes)
            );
            let _ = std::io::stderr().flush();
        }
    };
    let scan = ctx.engine.scan(&req, &CancelToken::new(), &sink)?;
    if tty {
        eprint!("\r{:60}\r", "");
    }
    Ok(scan)
}

fn scan_cmd(ctx: &Ctx, args: &ScanArgs) -> Result<(), Box<dyn std::error::Error>> {
    let scan = do_scan(ctx, &args.categories, args.min_age_hours)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&scan.summary())?);
        return Ok(());
    }
    print_scan(&scan, args.top);
    Ok(())
}

fn print_scan(scan: &Scan, top: usize) {
    println!(
        "Scanned {} entries in {:.2}s{}\n",
        scan.entries,
        scan.elapsed.as_secs_f64(),
        if scan.elevated { "" } else { "  (not elevated: some system locations are hidden)" }
    );
    println!("{:<32} {:<8} {:>11} {:>9}  NOTES", "CATEGORY", "RISK", "SIZE", "ITEMS");
    let mut total = 0u64;
    for (rule, cat) in scan.rules.iter().zip(&scan.categories) {
        let mut notes = Vec::new();
        match cat.status {
            CategoryStatus::NeedsAdmin => notes.push("needs administrator".to_string()),
            CategoryStatus::Unavailable => {
                notes.push(cat.note.clone().unwrap_or_else(|| "unavailable".into()))
            }
            _ => {}
        }
        if cat.stats.kept_files > 0 {
            notes.push(format!(
                "kept {} recent ({})",
                cat.stats.kept_files,
                human_bytes(cat.stats.kept_bytes)
            ));
        }
        if cat.stats.busy_files > 0 {
            notes.push(format!("{} in use", cat.stats.busy_files));
        }
        if cat.stats.links > 0 {
            notes.push(format!("{} links skipped", cat.stats.links));
        }
        if cat.stats.denied > 0 && cat.status == CategoryStatus::Found {
            notes.push("some locations need administrator".to_string());
        }
        if cat.stats.unreadable > 0 {
            notes.push(format!("{} couldn't be read", cat.stats.unreadable));
        }
        total += cat.stats.bytes;
        println!(
            "{:<32} {:<8} {:>11} {:>9}  {}",
            rule.name,
            risk_label(rule.risk),
            if cat.stats.bytes > 0 { human_bytes(cat.stats.bytes) } else { "—".into() },
            cat.stats.items,
            notes.join("; ")
        );
        for item in cat.items.iter().take(top) {
            println!("    {:>10}  {}", human_bytes(item.size), item.path.display());
        }
    }
    println!("{:<32} {:<8} {:>11}", "Total found", "", human_bytes(total));
    for n in &scan.notes {
        println!("note: {n}");
    }
}

fn clean_cmd(ctx: &Ctx, args: &CleanArgs) -> Result<(), Box<dyn std::error::Error>> {
    let scan = do_scan(ctx, &args.categories, args.min_age_hours)?;
    let selection = if args.categories.is_empty() {
        scan.default_selection()
    } else {
        Selection { categories: args.categories.iter().cloned().collect(), ..Selection::default() }
    };
    let policy = if args.undo {
        DeletionPolicy::AlwaysUndo
    } else if args.permanent {
        DeletionPolicy::AlwaysPermanent
    } else {
        ctx.settings.deletion_policy
    };
    let plan = scan.plan(&selection, policy);
    if plan.items == 0 {
        println!("Nothing to clean.");
        return Ok(());
    }
    println!("{:<32} {:>11} {:>9}  REMOVAL", "CATEGORY", "SIZE", "ITEMS");
    for p in &plan.categories {
        let name = scan.rules.iter().find(|r| r.id == p.id).map_or(p.id, |r| r.name);
        println!(
            "{:<32} {:>11} {:>9}  {}",
            name,
            human_bytes(p.bytes),
            p.items,
            if p.strategy == Strategy::Undoable { "recoverable" } else { "permanent" }
        );
    }
    println!(
        "\nTotal {} in {} items — {} recoverable, {} permanent.",
        human_bytes(plan.bytes),
        plan.items,
        human_bytes(plan.undo_bytes),
        human_bytes(plan.permanent_bytes)
    );
    if !args.yes {
        println!("\nThis was a dry run. Re-run with --yes to clean.");
        return Ok(());
    }
    let opts = CleanOptions { policy, undo_days: ctx.settings.undo_days, threads: None };
    let report = ctx.engine.clean(&scan, &selection, &opts, &CancelToken::new(), &NullSink)?;
    println!(
        "\nCleaned {} files ({}) in {:.1}s.",
        report.files,
        human_bytes(report.bytes),
        report.elapsed_ms as f64 / 1000.0
    );
    if let Some(b) = &report.batch {
        println!(
            "{} is recoverable until {} — `vexus-cli recovery restore {}` brings it back.",
            human_bytes(b.bytes),
            stamp(b.expires_at),
            b.id
        );
    }
    for (reason, stat) in &report.skipped {
        println!("skipped {} files: {}", stat.files, reason.label());
    }
    for c in &report.categories {
        for f in &c.failures {
            println!("failed: {} — {}", f.path, f.reason);
        }
    }
    let history = vexus_core::history::History::new(&ctx.data.state);
    let mut entry = vexus_core::history::HistoryEntry::new(
        vexus_core::history::HistoryKind::Clean,
        report.files,
        report.bytes,
    );
    entry.undo_bytes = report.undo_bytes;
    entry.categories = selection.categories.iter().cloned().collect();
    entry.batch = report.batch.as_ref().map(|b| b.id.clone());
    history.append(&entry);
    Ok(())
}

fn recovery_cmd(ctx: &Ctx, cmd: &RecoveryCmd) -> Result<(), Box<dyn std::error::Error>> {
    let store = ctx.engine.recovery();
    match cmd {
        RecoveryCmd::List => {
            let batches = store.list();
            if batches.is_empty() {
                println!("Nothing in Recovery.");
            }
            for b in batches {
                println!(
                    "{}  {:>9}  {:>6} files  expires {}  {}{}",
                    b.id,
                    human_bytes(b.bytes),
                    b.files,
                    stamp(b.expires_at),
                    b.label,
                    if b.complete { "" } else { "  (incomplete)" }
                );
            }
        }
        RecoveryCmd::Restore { id } => {
            let r = ctx.engine.restore(id, &CancelToken::new())?;
            println!(
                "Restored {} files ({}). {} already existed, {} refused, {} failed.{}",
                r.restored_files,
                human_bytes(r.restored_bytes),
                r.conflicts,
                r.refused,
                r.failed,
                if r.batch_removed { " Batch removed." } else { "" }
            );
        }
        RecoveryCmd::Purge { id, expired } => {
            if let Some(id) = id {
                println!("Freed {}.", human_bytes(store.purge(id)?));
            } else if *expired {
                let gone = store.purge_expired(now_unix());
                println!("Removed {} expired batches.", gone.len());
            } else {
                return Err("give a batch id, or --expired".into());
            }
        }
    }
    Ok(())
}
