//! android.* : Activity, View tree, Color, Log. View state is stored as host fields ("@...") on the
//! object itself, so app subclasses of framework views work with no extra bookkeeping.

use super::{arg, jstr};
use crate::{
    apk::Apk,
    arsc::Resources,
    axml::{self, Element, Value},
    bytes::Result,
    dex::Dex,
    vm::{Native, Object, Vm},
};
use std::collections::HashMap;

pub const CONTENT_VIEW: &str = "@contentView";
pub const CHILDREN: &str = "@children";
const ID: &str = "@id";
const CONTEXT: &str = "@context";
const TEXT: &str = "@text";
const TEXT_COLOR: &str = "@textColor";
const TEXT_SIZE: &str = "@textSize";
const BACKGROUND: &str = "@background";
const PADDING: [&str; 4] = ["@padLeft", "@padTop", "@padRight", "@padBottom"];
const ORIENTATION: &str = "@orientation";
const ON_CLICK: &str = "@onClick";
const LAYOUT_PARAMS: &str = "@layoutParams";
const WIDTH: &str = "@width";
const HEIGHT: &str = "@height";

/// A native that stores argument 1 into a host field of `this`.
macro_rules! setter {
    ($key:expr) => {
        (|vm: &mut Vm, a: &[u32]| {
            vm.set_field(arg(a, 0)?, $key, arg(a, 1)? as u64)?;
            Ok(0)
        }) as Native
    };
}

pub fn natives() -> Vec<(&'static str, Native)> {
    vec![
        ("Landroid/app/Activity;->onCreate(Landroid/os/Bundle;)V", |_, _| Ok(0)),
        ("Landroid/app/Activity;->setContentView(Landroid/view/View;)V", setter!(CONTENT_VIEW)),
        ("Landroid/app/Activity;->setContentView(I)V", |vm, a| {
            let act = arg(a, 0)?;
            let root = inflate_layout(vm, arg(a, 1)?, act)?;
            vm.set_field(act, CONTENT_VIEW, root as u64)?;
            Ok(0)
        }),
        ("Landroid/app/Activity;->findViewById(I)Landroid/view/View;", |vm, a| {
            let root = vm.field(arg(a, 0)?, CONTENT_VIEW)?.unwrap_or(0) as u32;
            Ok(find_view(vm, root, arg(a, 1)?, 0)? as u64)
        }),
        ("Landroid/view/View;->findViewById(I)Landroid/view/View;", |vm, a| Ok(find_view(vm, arg(a, 0)?, arg(a, 1)?, 0)? as u64)),
        ("Landroid/view/View;-><init>(Landroid/content/Context;)V", setter!(CONTEXT)),
        ("Landroid/view/View;->setBackgroundColor(I)V", setter!(BACKGROUND)),
        ("Landroid/view/View;->setOnClickListener(Landroid/view/View$OnClickListener;)V", setter!(ON_CLICK)),
        ("Landroid/view/View;->setPadding(IIII)V", |vm, a| {
            for (i, key) in PADDING.iter().enumerate() {
                vm.set_field(arg(a, 0)?, key, arg(a, i + 1)? as u64)?;
            }
            Ok(0)
        }),
        ("Landroid/widget/TextView;->setText(Ljava/lang/CharSequence;)V", |vm, a| {
            let text = arg(a, 1)?;
            if text != 0 && !matches!(vm.obj(text)?, Object::Str(_)) {
                // shortcut: only String is a CharSequence so far; add StringBuilder/Spannable when an app needs them
                return Err(format!("setText: unsupported CharSequence {}", vm.class_of(text)?));
            }
            vm.set_field(arg(a, 0)?, TEXT, text as u64)?;
            Ok(0)
        }),
        ("Landroid/widget/TextView;->setTextColor(I)V", setter!(TEXT_COLOR)),
        ("Landroid/widget/TextView;->setTextSize(F)V", setter!(TEXT_SIZE)),
        ("Landroid/widget/LinearLayout;->setOrientation(I)V", setter!(ORIENTATION)),
        ("Landroid/view/ViewGroup$LayoutParams;-><init>(II)V", |vm, a| {
            vm.set_field(arg(a, 0)?, WIDTH, arg(a, 1)? as u64)?;
            vm.set_field(arg(a, 0)?, HEIGHT, arg(a, 2)? as u64)?;
            Ok(0)
        }),
        ("Landroid/view/ViewGroup;->addView(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V", |vm, a| {
            let child = arg(a, 1)?;
            vm.set_field(child, LAYOUT_PARAMS, arg(a, 2)? as u64)?; // also rejects addView(null), like Android
            add_child(vm, arg(a, 0)?, child)?;
            Ok(0)
        }),
        ("Landroid/graphics/Color;->parseColor(Ljava/lang/String;)I", |vm, a| {
            let s = vm.string(arg(a, 0)?)?;
            parse_color(s).map(|c| c as u64).ok_or_else(|| format!("IllegalArgumentException: Unknown color {s:?}"))
        }),
        ("Landroid/util/Log;->v(Ljava/lang/String;Ljava/lang/String;)I", |vm, a| log(vm, a, 'V')),
        ("Landroid/util/Log;->d(Ljava/lang/String;Ljava/lang/String;)I", |vm, a| log(vm, a, 'D')),
        ("Landroid/util/Log;->i(Ljava/lang/String;Ljava/lang/String;)I", |vm, a| log(vm, a, 'I')),
        ("Landroid/util/Log;->w(Ljava/lang/String;Ljava/lang/String;)I", |vm, a| log(vm, a, 'W')),
        ("Landroid/util/Log;->e(Ljava/lang/String;Ljava/lang/String;)I", |vm, a| log(vm, a, 'E')),
    ]
}

/// Loads an APK and creates (but doesn't start) its launcher activity.
pub fn launch(path: &str) -> Result<(Vm, u32)> {
    let mut apk = Apk::open(path)?;
    let manifest = axml::parse(&apk.read("AndroidManifest.xml")?)?;
    let launcher = axml::launcher_activity(&manifest).ok_or("no launcher activity in manifest")?;
    let dexes = apk
        .dex_names()
        .iter()
        .map(|n| Dex::parse(&apk.read(n)?).map_err(|e| format!("{n}: {e}")))
        .collect::<Result<Vec<_>>>()?;
    let res = if apk.has("resources.arsc") { Some(Resources::parse(&apk.read("resources.arsc")?)?) } else { None };
    let mut vm = Vm::new(dexes);
    vm.res = res;
    vm.apk = Some(apk);
    let activity = vm.new_object(&format!("L{};", launcher.replace('.', "/")))?;
    Ok((vm, activity))
}

fn add_child(vm: &mut Vm, group: u32, child: u32) -> Result<()> {
    let list = match vm.field(group, CHILDREN)? {
        Some(l) => l as u32,
        None => {
            let l = vm.alloc(Object::Array(vec![]));
            vm.set_field(group, CHILDREN, l as u64)?;
            l
        }
    };
    if let Object::Array(v) = vm.obj_mut(list)? {
        v.push(child as u64);
    }
    Ok(())
}

fn children(vm: &Vm, v: u32) -> Result<Vec<u32>> {
    match vm.field(v, CHILDREN)? {
        None => Ok(vec![]),
        Some(list) => match vm.obj(list as u32)? {
            Object::Array(kids) => Ok(kids.iter().map(|&k| k as u32).collect()),
            _ => Err("children is not a list".into()),
        },
    }
}

/// Depth-first search for the view with android:id `id`; 0 (null) when absent, like Android.
fn find_view(vm: &Vm, v: u32, id: u32, depth: usize) -> Result<u32> {
    if v == 0 || depth > 64 {
        return Ok(0);
    }
    if vm.field(v, ID)? == Some(id as u64) {
        return Ok(v);
    }
    for c in children(vm, v)? {
        let found = find_view(vm, c, id, depth + 1)?;
        if found != 0 {
            return Ok(found);
        }
    }
    Ok(0)
}

fn inflate_layout(vm: &mut Vm, id: u32, ctx: u32) -> Result<u32> {
    let path = match vm.res.as_ref().and_then(|r| r.get(id)) {
        Some(Value::Str(p)) => p.clone(),
        _ => return Err(format!("Resources$NotFoundException: layout 0x{id:08x}")),
    };
    let xml = vm.apk.as_mut().ok_or("no APK loaded")?.read(&path)?;
    inflate(vm, &axml::parse(&xml)?, ctx, 0)
}

/// XML tag → class: bare names live in android.widget (a few in android.view), dotted names are app/library classes.
fn view_class(tag: &str) -> Result<String> {
    if matches!(tag, "merge" | "include" | "fragment" | "requestFocus" | "view") {
        return Err(format!("<{tag}> in layouts is not supported yet"));
    }
    Ok(match tag {
        t if t.contains('.') => format!("L{};", t.replace('.', "/")),
        "View" | "ViewGroup" | "ViewStub" | "SurfaceView" | "TextureView" => format!("Landroid/view/{tag};"),
        _ => format!("Landroid/widget/{tag};"),
    })
}

/// shortcut: one density for everything (1dp = 1sp = 1px); scale by the window's density once rendering exists.
fn px(v: &Value) -> Option<f32> {
    v.dimension().map(|(amount, _unit)| amount)
}

/// Builds the view for `el` and its children. Attributes go into the same host fields the Java setters use.
fn inflate(vm: &mut Vm, el: &Element, ctx: u32, depth: usize) -> Result<u32> {
    if depth > 64 {
        return Err("layout nested deeper than 64".into());
    }
    // shortcut: calls the (Context) constructor; custom views that only define (Context, AttributeSet) hit MISSING
    let v = vm.construct(&view_class(&el.name)?, "<init>(Landroid/content/Context;)V", &[ctx])?;
    let lp = vm.alloc(Object::Instance { class: "Landroid/view/ViewGroup$LayoutParams;".into(), fields: HashMap::new() });
    vm.set_field(lp, WIDTH, -2i32 as u32 as u64)?; // wrap_content unless the XML says otherwise
    vm.set_field(lp, HEIGHT, -2i32 as u32 as u64)?;
    vm.set_field(v, LAYOUT_PARAMS, lp as u64)?;

    for a in el.attrs.iter().filter(|a| a.ns.is_some()) {
        let val = match &a.value {
            Value::Ref(id) => vm.res.as_ref().and_then(|r| r.get(*id)).cloned().unwrap_or(a.value.clone()),
            other => other.clone(),
        };
        let int = |x: i32| x as u32 as u64;
        match (a.name.as_str(), &val) {
            ("id", _) => {
                if let Value::Ref(id) = a.value {
                    vm.set_field(v, ID, id as u64)?;
                }
            }
            ("text", Value::Str(s)) => {
                let s = vm.alloc(Object::Str(s.clone()));
                vm.set_field(v, TEXT, s as u64)?;
            }
            ("textSize", _) => if let Some(sp) = px(&val) { vm.set_field(v, TEXT_SIZE, sp.to_bits() as u64)? },
            ("textColor", _) => if let Some(c) = val.color() { vm.set_field(v, TEXT_COLOR, c as u64)? },
            // shortcut: only color backgrounds; drawable files are ignored until there is a renderer for them
            ("background", _) => if let Some(c) = val.color() { vm.set_field(v, BACKGROUND, c as u64)? },
            ("orientation", Value::Int(o)) => vm.set_field(v, ORIENTATION, int(*o))?,
            ("padding", _) => if let Some(p) = px(&val) { for key in PADDING { vm.set_field(v, key, int(p as i32))? } },
            ("paddingLeft" | "paddingTop" | "paddingRight" | "paddingBottom", _) => {
                let side = ["paddingLeft", "paddingTop", "paddingRight", "paddingBottom"].iter().position(|n| *n == a.name).unwrap();
                if let Some(p) = px(&val) { vm.set_field(v, PADDING[side], int(p as i32))? }
            }
            ("layout_width" | "layout_height", _) => {
                let key = if a.name == "layout_width" { WIDTH } else { HEIGHT };
                match val {
                    Value::Int(n) => vm.set_field(lp, key, int(n))?, // -1 match_parent, -2 wrap_content
                    _ => if let Some(p) = px(&val) { vm.set_field(lp, key, int(p as i32))? },
                }
            }
            _ => {} // shortcut: other attributes (margins, gravity, weight...) ignored until layout/rendering uses them
        }
    }
    for child in &el.children {
        let c = inflate(vm, child, ctx, depth + 1)?;
        add_child(vm, v, c)?;
    }
    Ok(v)
}

fn log(vm: &mut Vm, a: &[u32], level: char) -> Result<u64> {
    eprintln!("{level}/{}: {}", jstr(vm, arg(a, 0)?)?, jstr(vm, arg(a, 1)?)?);
    Ok(0)
}

/// `#RRGGBB` or `#AARRGGBB`, as android.graphics.Color.parseColor.
/// shortcut: named colors ("red", "blue"...) not supported yet.
fn parse_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#')?;
    let v = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(0xff00_0000 | v),
        8 => Some(v),
        _ => None,
    }
}

/// Text outline of the activity's view tree, one view per line.
pub fn dump_activity(vm: &Vm, activity: u32) -> Result<String> {
    let mut out = String::new();
    match vm.field(activity, CONTENT_VIEW)? {
        Some(root) => dump_view(vm, root as u32, 0, &mut out)?,
        None => out.push_str("(no content view)\n"),
    }
    Ok(out)
}

fn dump_view(vm: &Vm, v: u32, depth: usize, out: &mut String) -> Result<()> {
    if depth > 64 {
        return Err("view tree deeper than 64 (cycle?)".into());
    }
    let class = vm.class_of(v)?;
    let name = class.trim_end_matches(';').rsplit(['/', '$']).next().unwrap_or(&class);
    out.push_str(&format!("{:w$}{name}", "", w = depth * 2));
    let f = |key| vm.field(v, key);
    if let Some(id) = f(ID)? {
        out.push_str(&format!(" id=0x{id:08x}"));
    }
    if let Some(t) = f(TEXT)? {
        out.push_str(&format!(" text={:?}", jstr(vm, t as u32)?));
    }
    if let Some(s) = f(TEXT_SIZE)? {
        out.push_str(&format!(" size={}sp", f32::from_bits(s as u32)));
    }
    if let Some(c) = f(TEXT_COLOR)? {
        out.push_str(&format!(" color=#{:08x}", c as u32));
    }
    if let Some(c) = f(BACKGROUND)? {
        out.push_str(&format!(" bg=#{:08x}", c as u32));
    }
    if let Some(o) = f(ORIENTATION)? {
        out.push_str(if o == 1 { " vertical" } else { " horizontal" });
    }
    if f(PADDING[0])?.is_some() {
        let p = PADDING.map(|k| f(k).ok().flatten().unwrap_or(0) as i32);
        out.push_str(&format!(" padding={},{},{},{}", p[0], p[1], p[2], p[3]));
    }
    if let Some(lp) = f(LAYOUT_PARAMS)?.filter(|&lp| lp != 0) {
        let dim = |key| -> Result<String> {
            Ok(match vm.field(lp as u32, key)?.unwrap_or(0) as i32 {
                -1 => "match".into(),
                -2 => "wrap".into(),
                px => px.to_string(),
            })
        };
        out.push_str(&format!(" layout={},{}", dim(WIDTH)?, dim(HEIGHT)?));
    }
    if let Some(l) = f(ON_CLICK)?.filter(|&l| l != 0) {
        out.push_str(&format!(" onClick={}", vm.class_of(l as u32)?));
    }
    out.push('\n');
    for k in children(vm, v)? {
        dump_view(vm, k, depth + 1, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_color;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#0E1220"), Some(0xff0e1220));
        assert_eq!(parse_color("#800E1220"), Some(0x800e1220));
        assert_eq!(parse_color("0E1220"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#GGGGGG"), None);
    }
}
