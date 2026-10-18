//! Performance sanity check at scale. Ignored by default because it creates 100,000 files.
//!
//! ```text
//! cargo test -p vexus-core --release --test scale -- --ignored --nocapture
//! ```

use std::fs::{self, File};
use std::os::windows::fs::FileTimesExt;
use std::time::{Duration, Instant, SystemTime};

use vexus_core::Engine;
use vexus_core::clean::CleanOptions;
use vexus_core::env::Env;
use vexus_core::fsops::canonical_dir;
use vexus_core::model::DeletionPolicy;
use vexus_core::paths::{DataDirs, DataMode};
use vexus_core::progress::{CancelToken, NullSink};
use vexus_core::rules;
use vexus_core::scan::{ItemQuery, ItemSort, ScanRequest, Selection};

const DIRS: usize = 200;
const FILES_PER_DIR: usize = 500;

/// Makes a file look old. The age gate uses the newer of the modified and created times, so
/// both have to say so.
fn age(file: &File, when: SystemTime) {
    file.set_times(fs::FileTimes::new().set_modified(when).set_created(when)).unwrap();
}

#[test]
#[ignore = "creates 100,000 files; run manually with --ignored"]
fn scans_pages_searches_and_cleans_100k_items() {
    let dir = tempfile::tempdir().unwrap();
    let root = canonical_dir(dir.path()).unwrap();
    let env = Env::sandbox(&root);
    env.create_sandbox_skeleton().unwrap();
    let data = DataDirs::resolve(&DataMode::Custom(root.join(".vexus")), &env);
    data.ensure().unwrap();
    let engine = Engine::with_rules(env.clone(), rules::builtin(&env), &data, false);

    // A recent file in every folder stops folders collapsing, so every old file is its own item.
    let old = SystemTime::now() - Duration::from_secs(20 * 86_400);
    let t = Instant::now();
    for d in 0..DIRS {
        let folder = env.temp.join(format!("folder-{d:04}"));
        fs::create_dir_all(&folder).unwrap();
        File::create(folder.join("recent.tmp")).unwrap();
        for f in 0..FILES_PER_DIR {
            let file = File::create(folder.join(format!("file-{f:05}.tmp"))).unwrap();
            age(&file, old);
        }
    }
    println!("created {} files in {:.1}s", DIRS * (FILES_PER_DIR + 1), t.elapsed().as_secs_f64());

    let req =
        ScanRequest { categories: Some(vec!["windows.user-temp".into()]), ..Default::default() };
    let t = Instant::now();
    let scan = engine.scan(&req, &CancelToken::new(), &NullSink).unwrap();
    let scan_time = t.elapsed();
    let cat = &scan.categories[0];
    println!(
        "scan: {:.0} ms, {} items, {} entries examined",
        scan_time.as_secs_f64() * 1000.0,
        cat.items.len(),
        scan.entries
    );
    assert_eq!(cat.items.len(), DIRS * FILES_PER_DIR);
    assert_eq!(cat.stats.kept_files, DIRS as u64);

    let sel =
        Selection { categories: ["windows.user-temp".to_string()].into(), ..Default::default() };
    let t = Instant::now();
    let page = scan
        .items(
            "windows.user-temp",
            &ItemQuery { offset: 90_000, limit: 100, ..Default::default() },
            &sel,
        )
        .unwrap();
    println!(
        "page @90000: {:.1} ms ({} rows of {})",
        t.elapsed().as_secs_f64() * 1000.0,
        page.items.len(),
        page.total
    );

    let t = Instant::now();
    let page = scan
        .items(
            "windows.user-temp",
            &ItemQuery { search: Some("file-00042".into()), ..Default::default() },
            &sel,
        )
        .unwrap();
    println!(
        "search 'file-00042': {:.1} ms → {} matches",
        t.elapsed().as_secs_f64() * 1000.0,
        page.total
    );
    assert_eq!(page.total, DIRS);

    let t = Instant::now();
    let _ = scan
        .items(
            "windows.user-temp",
            &ItemQuery { sort: ItemSort::NameAsc, limit: 50, ..Default::default() },
            &sel,
        )
        .unwrap();
    println!("sort by name (100k): {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);

    let t = Instant::now();
    let plan = scan.plan(&sel, DeletionPolicy::Smart);
    println!("plan: {:.1} ms ({} items)", t.elapsed().as_secs_f64() * 1000.0, plan.items);

    let t = Instant::now();
    let report = engine
        .clean(
            &scan,
            &sel,
            &CleanOptions { threads: Some(8), ..CleanOptions::default() },
            &CancelToken::new(),
            &NullSink,
        )
        .unwrap();
    let secs = t.elapsed().as_secs_f64();
    println!(
        "clean: {:.1} s → {} files ({:.0} files/s)",
        secs,
        report.files,
        report.files as f64 / secs
    );
    assert_eq!(report.files, (DIRS * FILES_PER_DIR) as u64);
    // The recent files (and so their folders) must survive.
    assert_eq!(fs::read_dir(&env.temp).unwrap().count(), DIRS);
}

/// Where does the time go when deleting many small files? (Manual diagnostic.)
#[test]
#[ignore = "creates ~80,000 files; run manually with --ignored"]
fn delete_strategies_compared() {
    use rayon::prelude::*;
    use vexus_core::fsops::open_node;

    const N: usize = 20_000;
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let d = dir.path().join(name);
        fs::create_dir_all(&d).unwrap();
        (0..N)
            .map(|i| {
                let p = d.join(format!("f{i:06}.tmp"));
                File::create(&p).unwrap();
                p
            })
            .collect::<Vec<_>>()
    };

    let files = make("a");
    let t = Instant::now();
    for p in &files {
        fs::remove_file(p).unwrap();
    }
    println!(
        "A std::fs::remove_file, 1 thread      : {:>6.0} files/s",
        N as f64 / t.elapsed().as_secs_f64()
    );

    let files = make("b");
    let t = Instant::now();
    for p in &files {
        open_node(p, false).unwrap().delete().unwrap();
    }
    println!(
        "B open_node + delete via handle, 1 thr: {:>6.0} files/s",
        N as f64 / t.elapsed().as_secs_f64()
    );

    let files = make("c");
    let t = Instant::now();
    files.par_iter().for_each(|p| open_node(p, false).unwrap().delete().unwrap());
    println!(
        "C open_node + delete via handle, rayon: {:>6.0} files/s",
        N as f64 / t.elapsed().as_secs_f64()
    );

    let files = make("d");
    let t = Instant::now();
    files.par_iter().for_each(|p| fs::remove_file(p).unwrap());
    println!(
        "D std::fs::remove_file, rayon         : {:>6.0} files/s",
        N as f64 / t.elapsed().as_secs_f64()
    );

    let _ = make("e");
    let t = Instant::now();
    fs::remove_dir_all(dir.path().join("e")).unwrap();
    println!(
        "E std::fs::remove_dir_all (reference) : {:>6.0} files/s",
        N as f64 / t.elapsed().as_secs_f64()
    );
}

/// The full cleaner on 20,000 individually listed files, for comparison with the raw numbers above.
#[test]
#[ignore = "creates ~20,000 files; run manually with --ignored"]
fn engine_clean_throughput() {
    let dir = tempfile::tempdir().unwrap();
    let root = canonical_dir(dir.path()).unwrap();
    let env = Env::sandbox(&root);
    env.create_sandbox_skeleton().unwrap();
    let data = DataDirs::resolve(&DataMode::Custom(root.join(".vexus")), &env);
    data.ensure().unwrap();
    let engine = Engine::with_rules(env.clone(), rules::builtin(&env), &data, false);
    let old = SystemTime::now() - Duration::from_secs(20 * 86_400);
    for d in 0..40 {
        let folder = env.temp.join(format!("folder-{d:03}"));
        fs::create_dir_all(&folder).unwrap();
        File::create(folder.join("recent.tmp")).unwrap();
        for f in 0..500 {
            age(&File::create(folder.join(format!("f{f:05}.tmp"))).unwrap(), old);
        }
    }
    let req =
        ScanRequest { categories: Some(vec!["windows.user-temp".into()]), ..Default::default() };
    let scan = engine.scan(&req, &CancelToken::new(), &NullSink).unwrap();
    let sel =
        Selection { categories: ["windows.user-temp".to_string()].into(), ..Default::default() };
    for threads in [1usize, 4, 8] {
        // Re-create what the previous round removed so each round deletes the same amount.
        for d in 0..40 {
            let folder = env.temp.join(format!("folder-{d:03}"));
            for f in 0..500 {
                let p = folder.join(format!("f{f:05}.tmp"));
                if !p.exists() {
                    age(&File::create(&p).unwrap(), old);
                }
            }
        }
        let scan = engine.scan(&req, &CancelToken::new(), &NullSink).unwrap();
        let t = Instant::now();
        let report = engine
            .clean(
                &scan,
                &sel,
                &CleanOptions { threads: Some(threads), ..CleanOptions::default() },
                &CancelToken::new(),
                &NullSink,
            )
            .unwrap();
        println!(
            "engine clean, {threads} thread(s): {:>6.0} files/s ({} files)",
            report.files as f64 / t.elapsed().as_secs_f64(),
            report.files
        );
    }
    drop(scan);
}
