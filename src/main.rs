use apkloader::{apk::Apk, axml, bytes::Result, dex::Dex};
use std::process::ExitCode;

const USAGE: &str = "usage: apkloader <app.apk> [info|manifest|classes]";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (path, cmd) = match args.as_slice() {
        [_, p] => (p, "info"),
        [_, p, c] => (p, c.as_str()),
        _ => return Err(USAGE.into()),
    };
    let mut apk = Apk::open(path)?;
    let manifest = axml::parse(&apk.read("AndroidManifest.xml")?)?;
    let dexes = apk
        .dex_names()
        .iter()
        .map(|n| Dex::parse(&apk.read(n)?).map_err(|e| format!("{n}: {e}")))
        .collect::<Result<Vec<_>>>()?;

    match cmd {
        "info" => {
            println!("package:  {}", manifest.str_attr("package").unwrap_or("?"));
            println!("launcher: {}", axml::launcher_activity(&manifest).as_deref().unwrap_or("(none)"));
            let classes: usize = dexes.iter().map(|d| d.classes.len()).sum();
            let methods: usize = dexes.iter().map(|d| d.methods.len()).sum();
            println!("dex:      {} file(s), {classes} classes, {methods} method refs", dexes.len());
        }
        "manifest" => print!("{}", manifest.dump()),
        "classes" => {
            for d in &dexes {
                for c in &d.classes {
                    println!("{}", d.type_name(c.class));
                    for m in c.direct_methods.iter().chain(&c.virtual_methods) {
                        let code = m.code.as_ref().map_or("no code".into(), |c| format!("regs={} insns={}", c.registers, c.insns.len()));
                        println!("  {}  {code}", d.method_sig(m.method));
                    }
                }
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}
