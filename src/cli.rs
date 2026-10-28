use crate::{
    elevation::{self, Job, Update},
    engine::{self, Control, Plan},
    fail, handles, rules, scan, selection, Result,
};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
pub fn run() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let explicit = if let Some(i) = args.iter().position(|a| a == "--lang") {
        if args.iter().filter(|a| a.as_str() == "--lang").count() != 1 {
            return fail(
                "Invalid language option. Use --lang auto, en, zh-CN, zh-TW, ja, ko, or de.",
            );
        }
        let value = args
            .get(i + 1)
            .cloned()
            .ok_or("Missing --lang value. Choose auto, en, zh-CN, zh-TW, ja, ko, or de.")?;
        args.drain(i..=i + 1);
        Some(value)
    } else {
        None
    };
    let choice = crate::i18n::init(explicit.as_deref())?;
    if choice.source == "invalid_preference" {
        eprintln!("{}", crate::i18n::t("language.corrupt"));
    }
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    let command = &args[0];
    let value_flags = [
        "--scan",
        "--select",
        "--selection",
        "--destination",
        "--out",
        "--mode",
        "--state",
        "--source",
        "--target",
        "--plan",
        "--transaction",
        "--path",
        "--import",
        "--volumes",
    ];
    let mut boolean_flags = vec!["--yes", "--elevate", "--measure", "--batches"];
    // Occupancy actions are deferred (docs/requirements.md 2.1); only the debug
    // __force-test helper still accepts the flag.
    if cfg!(debug_assertions) {
        boolean_flags.push("--force-close-handles");
    }
    let mut seen = std::collections::HashSet::new();
    let mut i = 1;
    while i < args.len() {
        let flag = &args[i];
        if !seen.insert(flag.clone()) {
            return fail(crate::i18n::format("option.unknown", &[flag.clone()]));
        }
        if boolean_flags.contains(&flag.as_str()) {
            i += 1;
        } else if value_flags.contains(&flag.as_str()) {
            if args.get(i + 1).is_none() || args[i + 1].starts_with("--") {
                return fail(crate::i18n::format("msg.37f9824aeb", &[flag.clone()]));
            }
            i += 2;
        } else {
            return fail(crate::i18n::format("option.unknown", &[flag.clone()]));
        }
    }

    let value = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let path = |flag: &str| -> Result<PathBuf> {
        value(flag)
            .map(PathBuf::from)
            .ok_or_else(|| crate::i18n::format("msg.37f9824aeb", &[format!("{}", flag)]).into())
    };
    let store = value("--state")
        .map(PathBuf::from)
        .unwrap_or_else(engine::default_store);
    let yes = args.iter().any(|a| a == "--yes");
    let control = Control {
        ..Control::default()
    };
    #[cfg(debug_assertions)]
    let progress = |s: String| eprintln!("{s}");
    let elevate = args.iter().any(|a| a == "--elevate");
    let updates = |u| {
        if let Update::Progress(s) = u {
            eprintln!("{s}");
        }
    };
    if value("--select").is_some() && value("--selection").is_some() {
        return fail(crate::i18n::t("selection.invalid"));
    }
    match command.as_str() {
        "--version" | "version" => println!("SaveMyCDrive {}", env!("CARGO_PKG_VERSION")),
        "volumes" => println!("{}", serde_json::to_string_pretty(&scan::volumes())?),
        "scan" => {
            let roots = if let Some(p) = value("--path") {
                vec![PathBuf::from(p)]
            } else {
                value("--volumes")
                    .ok_or(crate::i18n::t("msg.317959053e"))?
                    .split(',')
                    .map(|v| {
                        PathBuf::from(if v.ends_with(':') {
                            format!("{v}\\")
                        } else {
                            v.into()
                        })
                    })
                    .collect()
            };
            let mode = value("--mode").unwrap_or_else(|| "migrate".into());
            let result = elevation::execute(
                &Job::Scan {
                    roots,
                    mode,
                    store,
                    measure: args.iter().any(|a| a == "--measure"),
                },
                elevate,
                &control,
                &updates,
            )?;
            output(&value("--out"), &result)?;
        }
        "plan" => {
            let report: scan::Report = serde_json::from_slice(&fs::read(path("--scan")?)?)?;
            if report.mode != "migrate" {
                return fail(crate::i18n::t("selection.invalid"));
            }
            let selected = if let Some(file) = value("--selection") {
                let bytes = fs::read(file)?;
                if bytes.len() > 1024 * 1024 {
                    return fail(crate::i18n::t("selection.limit"));
                }
                serde_json::from_slice::<selection::Selection>(&bytes)?
            } else {
                let ids = value("--select")
                    .ok_or(crate::i18n::t("msg.1d0f86d60a"))?
                    .split(',')
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                selection::Selection::from_ids(&report.rows, &ids)?
            };
            let planned = elevation::execute(
                &Job::Plan {
                    rows: report.rows,
                    selection: selected,
                    destination: path("--destination")?,
                    store,
                },
                elevate,
                &control,
                &updates,
            )?;
            let planned: selection::Planned = serde_json::from_value(planned)?;
            if !planned.retained.is_empty() {
                eprintln!("{}", crate::i18n::t("selection.retained"));
                for p in &planned.retained {
                    eprintln!("{}", p.display());
                }
            }
            let plan = planned.plan;
            output(&value("--out"), &plan)?;
        }
        "migrate" | "apply" => {
            let plan = if command == "migrate" {
                engine::custom_plan(&path("--source")?, &path("--target")?, &store)?
            } else {
                serde_json::from_slice::<Plan>(&fs::read(path("--plan")?)?)?
            };
            for i in &plan.items {
                eprintln!("{} -> {}", i.source.display(), i.target.display());
            }
            eprintln!("{}", crate::i18n::t("system.policy"));
            let sources: Vec<_> = plan.items.iter().map(|i| i.source.clone()).collect();
            let users = crate::handles::users(&sources);
            if !users.is_empty() {
                eprint!("{}", crate::handles::advice(&users));
            }
            confirm(yes)?;
            let records = elevation::execute(
                &Job::Migrate {
                    plan,
                    store,
                    options: control.options.clone(),
                },
                elevate,
                &control,
                &updates,
            )?;
            println!("{}", serde_json::to_string_pretty(&records)?);
        }
        "restore" => {
            let sources = if let Some(p) = value("--source") {
                vec![PathBuf::from(p)]
            } else {
                let r: scan::Report = serde_json::from_slice(&fs::read(path("--scan")?)?)?;
                let selection = if let Some(file) = value("--selection") {
                    let bytes = fs::read(file)?;
                    if bytes.len() > 1024 * 1024 {
                        return fail(crate::i18n::t("selection.limit"));
                    }
                    serde_json::from_slice::<selection::Selection>(&bytes)?
                } else {
                    let ids = value("--select")
                        .ok_or(crate::i18n::t("msg.0d3976df88"))?
                        .split(',')
                        .map(str::to_string)
                        .collect::<Vec<_>>();
                    selection::Selection::from_ids(&r.rows, &ids)?
                };
                selection.restore_sources(&r.rows)?
            };
            eprintln!("{}", crate::i18n::t("msg.5960ff2c33"));
            eprintln!("{}", crate::i18n::t("system.policy"));
            let users = engine::in_use(&sources);
            if !users.is_empty() {
                eprint!("{}", crate::handles::advice(&users));
            }
            confirm(yes)?;
            let result = elevation::execute(
                &Job::Restore {
                    sources,
                    store,
                    options: control.options.clone(),
                },
                elevate,
                &control,
                &updates,
            )?;
            for r in result.as_array().ok_or("Invalid worker result")? {
                println!("{}", serde_json::to_string(r)?);
            }
        }
        "history" => {
            if args.iter().any(|a| a == "--batches") {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&crate::batch::history(&store)?)?
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&engine::history(&store)?)?
                );
            }
            for dir in engine::leftover_dirs(&store)? {
                eprintln!(
                    "{}",
                    crate::i18n::format("history.leftover", &[dir.display().to_string()])
                );
            }
        }
        "recover" => {
            let id = value("--transaction").ok_or(crate::i18n::t("msg.99e1d53c23"))?;
            confirm(yes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&elevation::execute(
                    &Job::Recover {
                        id,
                        store,
                        options: control.options.clone()
                    },
                    elevate,
                    &control,
                    &updates
                )?)?
            );
        }
        "handles" => {
            let (rows, note) = handles::query(&[path("--path")?])?;
            if !note.is_empty() {
                eprintln!("{note}");
            }
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        "rules" => {
            if let Some(file) = value("--import") {
                let p = rules::import(Path::new(&file), &store)?;
                println!(
                    "{}",
                    crate::i18n::format("msg.15f28763a9", &[format!("{}", p.revision)])
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&rules::active(&store)?)?);
            }
        }
        #[cfg(debug_assertions)]
        "__lock-test" => {
            use std::os::windows::fs::OpenOptionsExt;
            let f = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(path("--path")?)?;
            let other = value("--target")
                .map(|p| {
                    fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .share_mode(0)
                        .open(p)
                })
                .transpose()?;
            println!("locked");
            io::stdout().flush()?;
            let mut s = String::new();
            io::stdin().read_line(&mut s)?;
            std::mem::forget(f);
            std::mem::forget(other);
        }
        #[cfg(debug_assertions)]
        "__junction-test" => {
            let source = path("--source")?;
            let target = path("--target")?;
            fixture_boundary(&source)?;
            fixture_boundary(&target)?;
            crate::platform::create_junction(&source, &target)?;
        }
        #[cfg(debug_assertions)]
        "__force-test" => {
            let root = path("--path")?;
            fixture_boundary(&root)?;
            if !yes || !args.iter().any(|a| a == "--force-close-handles") {
                return fail("Test helper requires explicit --yes --force-close-handles.");
            }
            let n = handles::force_close(
                &[root.clone()],
                &root
                    .parent()
                    .ok_or("fixture parent")?
                    .join("fixture-handles.jsonl"),
                &progress,
            )?;
            println!("{n}");
        }
        _ => {
            return fail(crate::i18n::format(
                "msg.1008985df9",
                &[format!("{}", command)],
            ))
        }
    }
    Ok(())
}
#[cfg(debug_assertions)]
fn fixture_boundary(path: &Path) -> Result<()> {
    let absolute = crate::platform::absolute(path)?;
    let parent = absolute.parent().ok_or("fixture parent")?;
    if fs::read(parent.join("savemycdrive-fixture.marker"))? != b"SaveMyCDrive test fixture\n" {
        return fail("Missing test-fixture marker.");
    }
    Ok(())
}
fn output<T: serde::Serialize>(path: &Option<String>, value: &T) -> Result<()> {
    if let Some(p) = path {
        crate::platform::atomic_json(Path::new(p), value)?;
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}
fn confirm(yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    eprint!("{}", crate::i18n::t("msg.c0fa62a422"));
    io::stderr().flush()?;
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    if s.trim() != "yes" {
        return fail(crate::i18n::t("msg.a37778f17c"));
    }
    Ok(())
}
fn print_help() {
    const COMMANDS: &str = r"smcd volumes
smcd scan --volumes C:,D: --mode migrate|restore [--measure] [--out scan.json]
smcd scan --path E:\test --mode migrate [--measure]
smcd plan --scan scan.json --select id1,id2 --destination D:\Dev --out plan.json
smcd plan --scan scan.json --selection selection.json --destination D:\Dev --out plan.json
smcd scan --volumes C: --elevate --out scan.json
smcd apply --plan plan.json --elevate
smcd apply --plan plan.json [--yes]
smcd migrate --source C:\cache --target D:\Dev\cache [--yes]
smcd restore --source C:\cache [--yes]
smcd restore --scan links.json --select id1,id2 [--yes]
smcd history [--state E:\smcd-state] [--batches]
smcd recover --transaction ID [--yes]
smcd handles --path C:\cache
smcd rules [--import rules.json]";
    print!("{}", crate::i18n::format("cli.help", &[COMMANDS.into()]));
}
