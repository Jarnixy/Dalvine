//! Checks our parsers against Google's own tools (dexdump, aapt2) on every APK in tests/fixtures.
//! A tool that isn't installed skips its test with a message instead of failing.

use apkloader::{apk::Apk, arsc::Resources, axml::{self, Value}, dex::Dex};
use std::{collections::BTreeSet, path::PathBuf, process::Command};

fn fixtures() -> Vec<PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir("tests/fixtures")
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "apk"))
        .collect();
    v.sort();
    assert!(!v.is_empty(), "no fixtures");
    v
}

/// $DEXDUMP / $AAPT2, then PATH, then the newest ~/Android/Sdk/build-tools.
fn tool(name: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(name.to_uppercase()) {
        return Some(p.into());
    }
    if Command::new(name).arg("--help").output().is_ok() {
        return Some(name.into());
    }
    let sdk = PathBuf::from(std::env::var_os("HOME")?).join("Android/Sdk/build-tools");
    let mut dirs: Vec<_> = std::fs::read_dir(sdk).ok()?.filter_map(|e| Some(e.ok()?.path().join(name))).collect();
    dirs.sort();
    dirs.into_iter().rev().find(|p| p.exists())
}

fn run(tool: &PathBuf, args: &[&str]) -> String {
    let out = Command::new(tool).args(args).output().unwrap();
    assert!(out.status.success(), "{} failed: {}", tool.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// "Lcls;->name(sig)V insns=N" for every method, from our parser.
fn ours(apk: &PathBuf) -> BTreeSet<String> {
    let mut a = Apk::open(apk).unwrap();
    let mut set = BTreeSet::new();
    for n in a.dex_names() {
        let d = Dex::parse(&a.read(&n).unwrap()).unwrap();
        for c in &d.classes {
            for m in c.direct_methods.iter().chain(&c.virtual_methods) {
                let insns = m.code.as_ref().map_or(0, |c| c.insns.len());
                set.insert(format!("{} insns={insns}", d.method_sig(m.method)));
            }
        }
    }
    set
}

/// The same set, scraped from `dexdump` text output.
fn theirs(dexdump: &PathBuf, apk: &PathBuf) -> BTreeSet<String> {
    let text = run(dexdump, &[apk.to_str().unwrap()]);
    let field = |l: &str| l.split_once(": '").map(|(_, v)| v.trim_end_matches('\'').to_string());
    let (mut set, mut class, mut in_methods) = (BTreeSet::new(), String::new(), false);
    let mut pending: Option<String> = None; // method waiting for its insns size line
    let mut name = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Class descriptor") {
            class = field(t).unwrap();
        } else if t.starts_with("Direct methods") || t.starts_with("Virtual methods") {
            in_methods = true;
        } else if t.starts_with("Static fields") || t.starts_with("Instance fields") || t.starts_with("source_file_idx") {
            in_methods = false;
        } else if in_methods && t.starts_with("name ") {
            set.extend(pending.take().map(|m| format!("{m} insns=0"))); // previous method had no code
            name = field(t).unwrap();
        } else if in_methods && t.starts_with("type ") {
            pending = Some(format!("{class}->{name}{}", field(t).unwrap()));
        } else if t.starts_with("insns size") {
            let n = t.split(':').nth(1).unwrap().split_whitespace().next().unwrap();
            set.insert(format!("{} insns={n}", pending.take().unwrap()));
        }
    }
    set.extend(pending.map(|m| format!("{m} insns=0")));
    set
}

#[test]
fn dex_matches_dexdump() {
    let Some(dexdump) = tool("dexdump") else { return eprintln!("SKIP: dexdump not found") };
    for apk in fixtures() {
        let (o, t) = (ours(&apk), theirs(&dexdump, &apk));
        assert!(!o.is_empty(), "{}: no methods parsed", apk.display());
        assert_eq!(o, t, "{}", apk.display());
    }
}

#[test]
fn manifest_matches_aapt2() {
    let Some(aapt2) = tool("aapt2") else { return eprintln!("SKIP: aapt2 not found") };
    for apk in fixtures() {
        let p = apk.to_str().unwrap();
        // element names in document order, e.g. "manifest uses-sdk application activity ..."
        let want: Vec<String> = run(&aapt2, &["dump", "xmltree", p, "--file", "AndroidManifest.xml"])
            .lines()
            .filter_map(|l| l.trim().strip_prefix("E: "))
            .map(|l| l.split(" (line=").next().unwrap().to_string())
            .collect();
        let m = axml::parse(&Apk::open(&apk).unwrap().read("AndroidManifest.xml").unwrap()).unwrap();
        let mut got = vec![];
        m.walk(0, &mut |_, e| got.push(e.name.clone()));
        assert_eq!(got, want, "{p}");
        assert!(axml::launcher_activity(&m).is_some(), "{p}: no launcher activity");
    }
}

#[test]
fn resources_match_aapt2() {
    let Some(aapt2) = tool("aapt2") else { return eprintln!("SKIP: aapt2 not found") };
    let mut total = 0;
    for apk in fixtures() {
        let p = apk.to_str().unwrap();
        let dump = run(&aapt2, &["dump", "resources", p]);
        let res = Resources::parse(&Apk::open(&apk).unwrap().read("resources.arsc").unwrap()).unwrap();
        // every "resource 0x7f..." aapt2 lists must resolve, and string values must match exactly
        let mut lines = dump.lines().map(str::trim).peekable();
        let mut count = 0;
        while let Some(l) = lines.next() {
            let Some(id) = l.strip_prefix("resource 0x") else { continue };
            let id = u32::from_str_radix(&id[..8], 16).unwrap();
            count += 1;
            let v = res.get(id).unwrap_or_else(|| panic!("{p}: 0x{id:08x} missing"));
            if let (Some(want), Value::Str(got)) = (lines.peek().and_then(|n| n.strip_prefix("() \"")), v) {
                assert_eq!(format!("{got}\""), want, "{p}: 0x{id:08x}");
            }
        }
        assert!(res.len() >= count, "{p}: aapt2 lists {count}, we have {}", res.len());
        total += count;
    }
    assert!(total > 0, "no fixture has resources, so nothing was compared");
}

#[test]
fn resource_values_decode() {
    let res = Resources::parse(&Apk::open("tests/fixtures/layout-inflation.apk").unwrap().read("resources.arsc").unwrap()).unwrap();
    assert_eq!(res.get(0x7f010000).and_then(Value::color), Some(0xff0e1220)); // color/bg
    assert_eq!(res.get(0x7f020000).and_then(Value::dimension), Some((24.0, 1))); // dimen/pad = 24dp
    assert_eq!(res.get(0x7f040000), Some(&Value::Str("res/layout/activity_main.xml".into())));
    assert_eq!(res.get(0x7f050001), Some(&Value::Str("Hello from inflated XML".into())));
    assert_eq!(res.get(0x7f990000), None);
}

#[test]
fn hello_launcher() {
    let m = axml::parse(&Apk::open("tests/fixtures/hello.apk").unwrap().read("AndroidManifest.xml").unwrap()).unwrap();
    assert_eq!(m.str_attr("package"), Some("com.apkverse.demo"));
    assert_eq!(axml::launcher_activity(&m).as_deref(), Some("com.apkverse.demo.MainActivity"));
}

#[test]
fn layout_xml_parses() {
    // compiled res/layout/*.xml use the same binary format as the manifest
    let p = "tests/fixtures/layout-inflation.apk";
    let Some(aapt2) = tool("aapt2") else { return eprintln!("SKIP: aapt2 not found") };
    let want = run(&aapt2, &["dump", "xmltree", p, "--file", "res/layout/activity_main.xml"]).matches("E: ").count();
    let root = axml::parse(&Apk::open(p).unwrap().read("res/layout/activity_main.xml").unwrap()).unwrap();
    let mut got = 0;
    root.walk(0, &mut |_, _| got += 1);
    assert_eq!(got, want);
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    for len in [0, 3, 8, 0x70, 300] {
        let mut b = vec![0xffu8; len];
        if len >= 8 {
            b[..8].copy_from_slice(b"dex\n035\0");
        }
        assert!(Dex::parse(&b).is_err());
        assert!(axml::parse(&b).is_err());
        assert!(Resources::parse(&b).is_err());
    }
}
