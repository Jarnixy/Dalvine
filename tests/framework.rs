//! Real apps' onCreate() running against our fake framework.

use apkloader::runtime::android::{dump_activity, launch};

fn run_on_create(path: &str) -> Result<String, String> {
    let (mut vm, act) = launch(path)?;
    vm.invoke_virtual("onCreate(Landroid/os/Bundle;)V", &[act, 0])?;
    dump_activity(&vm, act)
}

#[test]
fn hello_on_create_builds_view_tree() {
    assert_eq!(
        run_on_create("tests/fixtures/hello.apk").unwrap(),
        "LinearLayout bg=#ff0e1220 vertical padding=40,120,40,40\n\
         \x20 TextView text=\"Hello from real DEX bytecode\" size=20sp color=#ffffffff layout=match,wrap\n\
         \x20 Button text=\"Run DEX click handler\" layout=match,wrap onClick=Lcom/apkverse/demo/MainActivity;\n"
    );
}

#[test]
fn xml_layout_is_inflated() {
    // compare: aapt2 dump xmltree tests/fixtures/layout-inflation.apk --file res/layout/activity_main.xml
    assert_eq!(
        run_on_create("tests/fixtures/layout-inflation.apk").unwrap(),
        "LinearLayout bg=#ff0e1220 vertical padding=24,24,24,24 layout=match,match\n\
         \x20 TextView id=0x7f030001 text=\"Hello from inflated XML\" size=20sp color=#ffffffff layout=match,wrap\n\
         \x20 Button id=0x7f030000 text=\"Tap (inflated button)\" layout=match,wrap onClick=Lcom/apkverse/layoutfixture/LayoutActivity;\n"
    );
}

#[test]
fn unimplemented_api_names_the_exact_signature() {
    let e = run_on_create("tests/fixtures/aw-probe.apk").unwrap_err();
    assert!(e.starts_with("MISSING: Lorg/androidweb/probe/MainActivity;->getSharedPreferences("), "{e}");
}
