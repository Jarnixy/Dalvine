//! Real apps' onCreate() running against our fake framework.

use apkloader::{
    runtime::android::{dump_activity, finished, launch, start, stop},
    ui,
};

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

/// The full UI path: layout → hit-test a screen point → app onClick → new text.
#[test]
fn taps_on_screen_run_app_click_handlers() {
    let Ok(font) = ui::load_font() else { return eprintln!("SKIP: no font (set DALVINE_FONT)") };
    let cases = [
        ("tests/fixtures/hello.apk", "DEX handler fired 2 time(s)"),
        ("tests/fixtures/layout-inflation.apk", "Inflated XML + DEX click 2"),
    ];
    for (path, want) in cases {
        let (mut vm, act) = launch(path).unwrap();
        start(&mut vm, act).unwrap();
        let placed = ui::layout(&vm, &font, act, 480, 800).unwrap();
        let [root, label, button] = placed[..] else { panic!("{path}: expected 3 views, got {placed:?}") };
        assert_eq!((root.x, root.y, root.w, root.h), (0, 0, 480, 800), "{path}: content view fills the window");
        assert!(label.y + label.h < button.y, "{path}: margin between label and button");
        assert_eq!(button.h, 48, "{path}: Android's minimum button height");

        assert!(!ui::tap(&mut vm, &placed, 1, 799).unwrap(), "{path}: empty space isn't clickable");
        let (cx, cy) = (button.x + button.w / 2, button.y + button.h / 2);
        for _ in 0..2 {
            assert!(ui::tap(&mut vm, &placed, cx, cy).unwrap(), "{path}: button hit");
        }
        let text = vm.field(label.view, "@text").unwrap().unwrap() as u32;
        assert_eq!(vm.string(text).unwrap(), want, "{path}");
    }
}

#[test]
fn back_button_finishes_the_activity() {
    let (mut vm, act) = launch("tests/fixtures/hello.apk").unwrap();
    start(&mut vm, act).unwrap();
    assert!(!finished(&vm, act).unwrap());
    vm.invoke_virtual("onBackPressed()V", &[act]).unwrap(); // app override calls super.onBackPressed()
    assert!(finished(&vm, act).unwrap());
    stop(&mut vm, act).unwrap();
}
