//! android.* : Activity, View tree, Color, Log. View state is stored as host fields ("@...") on the
//! object itself, so app subclasses of framework views work with no extra bookkeeping.

use super::{arg, jstr};
use crate::{
    bytes::Result,
    vm::{Native, Object, Vm},
};

pub const CONTENT_VIEW: &str = "@contentView";
pub const CHILDREN: &str = "@children";
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
            let (group, child) = (arg(a, 0)?, arg(a, 1)?);
            vm.obj(child)?; // addView(null) throws on Android too
            vm.set_field(child, LAYOUT_PARAMS, arg(a, 2)? as u64)?;
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
    if let Some(list) = f(CHILDREN)? {
        let Object::Array(kids) = vm.obj(list as u32)? else { return Err("children is not a list".into()) };
        for &k in kids {
            dump_view(vm, k as u32, depth + 1, out)?;
        }
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
