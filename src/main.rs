use apkloader::{
    apk::Apk,
    axml,
    bytes::Result,
    dex::Dex,
    runtime::android::{dump_activity, finished, launch, start, stop},
    ui,
    vm::Vm,
};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use std::process::ExitCode;

const USAGE: &str = "usage: apkloader <app.apk> [info | manifest | classes | tree | run | shot <out.ppm> [x,y ...]]";
/// Window size in px. shortcut: fixed phone-like size at density 1; make resizable with real dp scaling later.
const W: usize = 480;
const H: usize = 800;

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
    let path = args.get(1).ok_or(USAGE)?;
    let cmd = args.get(2).map_or("info", |c| c.as_str());
    let rest = args.get(3..).unwrap_or_default();

    match cmd {
        "info" | "manifest" | "classes" => inspect(path, cmd),
        "tree" => {
            let (mut vm, act) = launch(path)?;
            start(&mut vm, act)?;
            print!("{}", dump_activity(&vm, act)?);
            Ok(())
        }
        "shot" => {
            let [out, taps @ ..] = rest else { return Err(USAGE.into()) };
            let font = ui::load_font()?;
            let (mut vm, act) = launch(path)?;
            start(&mut vm, act)?;
            for t in taps {
                let (x, y) = t.split_once(',').and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?))).ok_or(USAGE)?;
                let placed = ui::layout(&vm, &font, act, W as i32, H as i32)?;
                if !ui::tap(&mut vm, &placed, x, y)? {
                    eprintln!("tap {x},{y}: nothing clickable there");
                }
            }
            let placed = ui::layout(&vm, &font, act, W as i32, H as i32)?;
            std::fs::write(out, ui::ppm(&ui::paint(&vm, &font, &placed, W, H)?, W, H)).map_err(|e| format!("{out}: {e}"))
        }
        "run" => window(path),
        _ => Err(USAGE.into()),
    }
}

/// Opens the app in a window: click = tap, Esc = back button.
fn window(path: &str) -> Result<()> {
    let font = ui::load_font()?;
    let (mut vm, act) = launch(path)?;
    start(&mut vm, act)?;
    let title = format!("Dalvine - {}", vm.class_of(act)?.trim_start_matches('L').trim_end_matches(';').replace('/', "."));
    let mut win = Window::new(&title, W, H, WindowOptions::default()).map_err(|e| format!("cannot open window: {e}"))?;
    win.set_target_fps(60);

    let mut frame = render(&vm, &font, act)?;
    let mut was_down = false;
    while win.is_open() && !finished(&vm, act)? {
        let down = win.get_mouse_down(MouseButton::Left);
        let mut changed = false;
        if was_down && !down {
            // tap on release, like Android
            if let Some((x, y)) = win.get_mouse_pos(MouseMode::Discard) {
                report(ui::tap(&mut vm, &frame.0, x as i32, y as i32));
                changed = true;
            }
        }
        was_down = down;
        if win.is_key_pressed(Key::Escape, KeyRepeat::No) {
            report(vm.invoke_virtual("onBackPressed()V", &[act]));
            changed = true;
        }
        if changed {
            frame = render(&vm, &font, act)?;
        }
        win.update_with_buffer(&frame.1, W, H).map_err(|e| e.to_string())?;
    }
    stop(&mut vm, act)
}

fn render(vm: &Vm, font: &fontdue::Font, act: u32) -> Result<(Vec<ui::Placed>, Vec<u32>)> {
    let placed = ui::layout(vm, font, act, W as i32, H as i32)?;
    let pixels = ui::paint(vm, font, &placed, W, H)?;
    Ok((placed, pixels))
}

/// An exception in an event handler would crash the app on Android; here it's printed and the window stays up.
fn report<T>(r: Result<T>) {
    if let Err(e) = r {
        eprintln!("app error: {e}");
    }
}

fn inspect(path: &str, cmd: &str) -> Result<()> {
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
        _ => {
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
    }
    Ok(())
}
