//! Developer tool: prints what AppReestr would show, with the reasons. Used to check the
//! classification on real systems and to attach to bug reports.

use appreestr_core::config::ConfigStore;
use appreestr_core::model::{App, Visibility};
use appreestr_core::reasons::{describe, format_size, source_short};
use appreestr_core::rules::Rules;
use appreestr_core::scan::{ScanInput, scan};
use appreestr_core::sysroot::SysRoot;

fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    if has("--help") || has("-h") {
        println!(
            "usage: appreestr-dump [--json] [--timings] [--all] [--plan PKG...] [--force-plan PKG...] [--leftovers APP-ID...]"
        );
        println!("  --all      also list console and system entries (not just counts)");
        return;
    }

    let sys = SysRoot::host();
    let store = ConfigStore::new(&sys);
    let rules = Rules::load(&store.rules_file());
    let settings = store.settings();
    let marks = store.overrides().as_map();
    let res = scan(&ScanInput { sys: &sys, rules: &rules, settings: &settings, marks: &marks });

    if let Some(i) = args.iter().position(|a| a == "--plan") {
        let targets: Vec<String> = args[i + 1..].to_vec();
        let prot = res.protection(&sys, &rules);
        println!("polkit agent running: {}", prot.polkit_agent);
        let mut h = appreestr_core::plan::open_local("/", "/var/lib/pacman").expect("alpm");
        match appreestr_core::plan::plan(&mut h, &targets, true, Some(&prot)) {
            Ok(p) => println!("{p:#?}"),
            Err(e) => println!("error: {e}"),
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--force-plan") {
        // What the Danger Zone would do (read-only): dependencies ignored, protection only reported.
        let targets: Vec<String> = args[i + 1..].to_vec();
        let prot = res.protection(&sys, &rules);
        let mut h = appreestr_core::plan::open_local("/", "/var/lib/pacman").expect("alpm");
        match appreestr_core::plan::plan_force(&mut h, &targets, false, Some(&prot)) {
            Ok(p) => println!("{p:#?}"),
            Err(e) => println!("error: {e}"),
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--leftovers") {
        for id in &args[i + 1..] {
            let Some(app) = res.apps.iter().find(|a| &a.id == id) else {
                println!("{id}: not found");
                continue;
            };
            let others: Vec<&App> = res.apps.iter().filter(|a| a.id != app.id).collect();
            println!("== {} ({id})", app.name);
            for l in appreestr_core::leftovers::find(&sys, app, &others) {
                println!(
                    "  {:?} {:?} {:>10} {}",
                    l.confidence,
                    l.kind,
                    l.size.map(format_size).unwrap_or("?".into()),
                    l.path.display()
                );
            }
        }
        return;
    }
    if has("--json") {
        println!("{}", serde_json::to_string_pretty(&res.apps).unwrap());
        return;
    }
    for w in &res.warnings {
        eprintln!("warning: {w}");
    }
    if has("--timings") {
        for (name, d) in &res.timings {
            println!("{name:>10}: {:>7.1} ms", d.as_secs_f64() * 1000.0);
        }
        println!();
    }
    for (vis, title) in [
        (Visibility::App, "APPLICATIONS"),
        (Visibility::Console, "CONSOLE"),
        (Visibility::Addon, "ADD-ONS"),
        (Visibility::System, "SYSTEM"),
    ] {
        let apps: Vec<&App> = res.apps.iter().filter(|a| a.visibility == vis).collect();
        println!("== {title} ({})", apps.len());
        if vis != Visibility::App && !has("--all") {
            continue;
        }
        for a in apps {
            print_app(a);
        }
        println!();
    }
}

fn print_app(a: &App) {
    let sources: Vec<String> =
        a.installs.iter().map(|i| format!("{} [{}]", source_short(&i.source), i.unit_name)).collect();
    let size = a.total_size().map(format_size).unwrap_or_default();
    println!("  {:<34} {:>9}  {}", truncate(&a.name, 34), size, sources.join(" + "));
    let reasons: Vec<String> = a.reasons.iter().map(describe).collect();
    println!("  {:<34}            · {}", "", reasons.join("; "));
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "…" }
}
