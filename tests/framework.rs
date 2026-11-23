//! Milestone 5: real app onCreate() runs against our fake framework.

use apkloader::{apk::Apk, axml, dex::Dex, runtime::android::dump_activity, vm::Vm};

/// Loads an APK and creates its launcher activity, like `apkloader <apk> run`.
fn launch(path: &str) -> (Vm, u32) {
    let mut apk = Apk::open(path).unwrap();
    let manifest = axml::parse(&apk.read("AndroidManifest.xml").unwrap()).unwrap();
    let dexes = apk.dex_names().iter().map(|n| Dex::parse(&apk.read(n).unwrap()).unwrap()).collect();
    let mut vm = Vm::new(dexes);
    let class = format!("L{};", axml::launcher_activity(&manifest).unwrap().replace('.', "/"));
    let activity = vm.new_object(&class).unwrap();
    (vm, activity)
}

#[test]
fn hello_on_create_builds_view_tree() {
    let (mut vm, act) = launch("tests/fixtures/hello.apk");
    vm.invoke_virtual("onCreate(Landroid/os/Bundle;)V", &[act, 0]).unwrap();
    assert_eq!(
        dump_activity(&vm, act).unwrap(),
        "LinearLayout bg=#ff0e1220 vertical padding=40,120,40,40\n\
         \x20 TextView text=\"Hello from real DEX bytecode\" size=20sp color=#ffffffff layout=match,wrap\n\
         \x20 Button text=\"Run DEX click handler\" layout=match,wrap onClick=Lcom/apkverse/demo/MainActivity;\n"
    );
}

#[test]
fn unimplemented_api_names_the_exact_signature() {
    let (mut vm, act) = launch("tests/fixtures/layout-inflation.apk");
    let e = vm.invoke_virtual("onCreate(Landroid/os/Bundle;)V", &[act, 0]).unwrap_err();
    assert!(e.starts_with("MISSING: Lcom/apkverse/layoutfixture/LayoutActivity;->setContentView(I)V"), "{e}");
}
