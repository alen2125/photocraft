//! Layer › Layer Color visibility toggles: one command per colour label that shows or hides every
//! layer marked with that colour, to bind to a key (Edit › Keyboard Shortcuts). Hides them when
//! all are visible, otherwise shows them (and the groups around them, so they really appear).

use photocraft_doc::LabelColor;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The colours that get a toggle (every label but "No Color").
const COLORS: [(LabelColor, &str, &str); 11] = [
    (LabelColor::Red, "layer.label.toggleRed", "Toggle Red Layers"),
    (LabelColor::Orange, "layer.label.toggleOrange", "Toggle Orange Layers"),
    (LabelColor::Yellow, "layer.label.toggleYellow", "Toggle Yellow Layers"),
    (LabelColor::Green, "layer.label.toggleGreen", "Toggle Green Layers"),
    (LabelColor::Seafoam, "layer.label.toggleSeafoam", "Toggle Seafoam Layers"),
    (LabelColor::Blue, "layer.label.toggleBlue", "Toggle Blue Layers"),
    (LabelColor::Indigo, "layer.label.toggleIndigo", "Toggle Indigo Layers"),
    (LabelColor::Magenta, "layer.label.toggleMagenta", "Toggle Magenta Layers"),
    (LabelColor::Fuchsia, "layer.label.toggleFuchsia", "Toggle Fuchsia Layers"),
    (LabelColor::Violet, "layer.label.toggleViolet", "Toggle Violet Layers"),
    (LabelColor::Gray, "layer.label.toggleGray", "Toggle Gray Layers"),
];

/// Whether `id` is one of the toggle commands (the shortcuts list shows them without a menu).
pub fn is_toggle(id: &str) -> bool {
    COLORS.iter().any(|(_, i, _)| *i == id)
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn toggle(s: &mut Session, color: LabelColor, title: &str) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let walk = st.doc.walk();
    let marked: Vec<_> = walk.iter().filter(|(_, _, l)| l.label == color).map(|(p, _, l)| (p.clone(), l.visible)).collect();
    if marked.is_empty() {
        return Err(EngineError::Other(format!("no layer is marked {}", color.label())));
    }
    let show = marked.iter().any(|(_, visible)| !visible);
    // Showing also shows the groups around a layer, or it would stay hidden with them.
    let mut paths: Vec<Vec<usize>> = Vec::new();
    for (path, _) in &marked {
        paths.push(path.clone());
        if show {
            paths.extend((1..path.len()).filter_map(|n| path.get(..n).map(<[usize]>::to_vec)));
        }
    }
    s.edit(title, |doc, _| {
        for p in &paths {
            if let Some(l) = doc.layer_at_mut(p) {
                l.visible = show;
            }
        }
        Ok(())
    })?;
    Ok(json!({"visible": show, "layers": marked.len()}))
}

pub fn specs() -> Vec<CommandSpec> {
    COLORS
        .iter()
        .map(|&(color, id, label)| {
            // `run` is a plain function pointer: one thin function per colour.
            let run: fn(&mut Session, &Value) -> Result<Value> = match color {
                LabelColor::Red => |s, _| toggle(s, LabelColor::Red, "Toggle Red Layers"),
                LabelColor::Orange => |s, _| toggle(s, LabelColor::Orange, "Toggle Orange Layers"),
                LabelColor::Yellow => |s, _| toggle(s, LabelColor::Yellow, "Toggle Yellow Layers"),
                LabelColor::Green => |s, _| toggle(s, LabelColor::Green, "Toggle Green Layers"),
                LabelColor::Seafoam => |s, _| toggle(s, LabelColor::Seafoam, "Toggle Seafoam Layers"),
                LabelColor::Blue => |s, _| toggle(s, LabelColor::Blue, "Toggle Blue Layers"),
                LabelColor::Indigo => |s, _| toggle(s, LabelColor::Indigo, "Toggle Indigo Layers"),
                LabelColor::Magenta => |s, _| toggle(s, LabelColor::Magenta, "Toggle Magenta Layers"),
                LabelColor::Fuchsia => |s, _| toggle(s, LabelColor::Fuchsia, "Toggle Fuchsia Layers"),
                LabelColor::Violet => |s, _| toggle(s, LabelColor::Violet, "Toggle Violet Layers"),
                LabelColor::Gray | LabelColor::None => |s, _| toggle(s, LabelColor::Gray, "Toggle Gray Layers"),
            };
            CommandSpec {
                id,
                label,
                menu: &[],
                shortcut: None,
                params: "{} → {\"visible\":bool,\"layers\":n} (shows every layer marked with this colour, or hides them when all are shown; bind a key in Keyboard Shortcuts)",
                enabled,
                run,
                journal: true,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::LayerId;

    fn session() -> (Session, [LayerId; 3]) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
        let ids = [(); 3].map(|_| LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap()));
        (s, ids)
    }

    fn mark(s: &mut Session, id: LayerId, color: &str) {
        s.execute("layer.setLabelColor", json!({"layer": id.0, "color": color})).unwrap();
    }

    fn visible(s: &Session, id: LayerId) -> bool {
        s.active().unwrap().doc.layer(id).unwrap().visible
    }

    #[test]
    fn toggles_every_layer_of_the_colour() {
        let (mut s, [a, b, c]) = session();
        mark(&mut s, a, "red");
        mark(&mut s, b, "red");
        mark(&mut s, c, "blue");
        assert_eq!(s.execute("layer.label.toggleRed", json!({})).unwrap()["visible"], false);
        assert!(!visible(&s, a) && !visible(&s, b) && visible(&s, c));
        assert_eq!(s.execute("layer.label.toggleRed", json!({})).unwrap()["visible"], true);
        assert!(visible(&s, a) && visible(&s, b) && visible(&s, c));
    }

    #[test]
    fn a_mixed_state_shows_them_all_and_undoes() {
        let (mut s, [a, b, _]) = session();
        mark(&mut s, a, "green");
        mark(&mut s, b, "green");
        s.execute("layer.setProps", json!({"layer": a.0, "visible": false})).unwrap();
        s.execute("layer.label.toggleGreen", json!({})).unwrap();
        assert!(visible(&s, a) && visible(&s, b));
        s.undo();
        assert!(!visible(&s, a) && visible(&s, b));
    }

    #[test]
    fn showing_a_layer_shows_its_hidden_group() {
        let (mut s, [a, _, _]) = session();
        mark(&mut s, a, "violet");
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.groupLayers", json!({})).unwrap();
        let group = s.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.is_group()).map(|(_, _, l)| l.id).unwrap();
        s.execute("layer.setProps", json!({"layer": a.0, "visible": false})).unwrap();
        s.execute("layer.setProps", json!({"layer": group.0, "visible": false})).unwrap();
        s.execute("layer.label.toggleViolet", json!({})).unwrap();
        assert!(visible(&s, a) && visible(&s, group));
    }

    #[test]
    fn errors_without_marked_layers_or_a_document() {
        let (mut s, _) = session();
        assert!(s.execute("layer.label.toggleRed", json!({"junk": 1})).is_err());
        let mut empty = Session::new();
        assert!(empty.execute("layer.label.toggleBlue", json!({})).is_err());
    }

    #[test]
    fn every_colour_has_its_command() {
        for (color, id, _) in COLORS {
            assert!(is_toggle(id));
            assert!(crate::commands::find(id).is_some(), "{id}");
            assert_ne!(color, LabelColor::None);
        }
    }
}
