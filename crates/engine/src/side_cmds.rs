//! Parallel layers: `layer.parallelLayer` flags layers so they leave the document's composite and
//! show on the side canvas beside it (see `photocraft_doc::Document::side_view`).

use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.filter(|id| d.doc.layer(*id).is_some()).map(|_| ()).ok_or_else(|| "no active layer".into())
}

/// Set or toggle the parallel flag on the target layers, in one history step. With no `side`
/// param the flag is toggled: turned on unless every target already has it.
fn parallel_layer(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.parallelLayer";
    let want = match p.get("side") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(v) => return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("`side` = {v} must be a boolean") }),
    };
    let ids: Vec<LayerId> = crate::layer_multi_cmds::targets(s, p)?;
    let on = s.edit("Parallel Layer", |doc, _| {
        let on = want.unwrap_or_else(|| !ids.iter().all(|id| doc.layer(*id).is_some_and(|l| l.side)));
        for id in &ids {
            doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?.side = on;
        }
        Ok(on)
    })?;
    Ok(json!({"side": on}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "layer.parallelLayer",
        label: "Parallel Layer",
        menu: &["Layer"],
        shortcut: None,
        params: r##"{"layer":id?,"side":bool? (absent: toggle)} → {"side":bool} (a parallel layer is left out of the document and shown, with every layer below it, on the side canvas beside it)"##,
        enabled: has_layer,
        run: parallel_layer,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::Rect;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100, "height": 80})).unwrap();
        s
    }

    fn new_layer(s: &mut Session) -> LayerId {
        let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        s.edit("paint", |doc, _| {
            doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        id
    }

    fn flag(s: &Session, id: LayerId) -> bool {
        s.active().unwrap().doc.layer(id).unwrap().side
    }

    #[test]
    fn toggles_and_undoes() {
        let mut s = session();
        let id = new_layer(&mut s);
        assert_eq!(s.execute("layer.parallelLayer", json!({})).unwrap()["side"], true);
        assert!(flag(&s, id));
        s.undo();
        assert!(!flag(&s, id));
        s.redo();
        assert!(flag(&s, id));
        assert_eq!(s.execute("layer.parallelLayer", json!({})).unwrap()["side"], false);
        assert!(!flag(&s, id));
    }

    #[test]
    fn explicit_value_and_layer() {
        let mut s = session();
        let a = new_layer(&mut s);
        let b = new_layer(&mut s);
        s.execute("layer.parallelLayer", json!({"layer": a.0, "side": true})).unwrap();
        s.execute("layer.parallelLayer", json!({"layer": a.0, "side": true})).unwrap();
        assert!(flag(&s, a) && !flag(&s, b));
    }

    #[test]
    fn side_layer_leaves_the_document_composite() {
        let mut s = session();
        let id = new_layer(&mut s);
        s.execute("layer.parallelLayer", json!({"layer": id.0, "side": true})).unwrap();
        let doc = &s.active().unwrap().doc;
        let main = doc.main_view().unwrap();
        let rect = Rect::new(0, 0, 10, 10);
        let with = photocraft_compose::render(&main, rect).px[0];
        let side = photocraft_compose::render(&doc.side_view().unwrap(), rect).px[0];
        assert_ne!(with, side, "the red square shows on the side canvas only");
        assert_eq!(side[0], 1.0);
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = session();
        new_layer(&mut s);
        assert!(s.execute("layer.parallelLayer", json!({"side": "yes"})).is_err());
        assert!(s.execute("layer.parallelLayer", json!({"layer": 999_999_999u64})).is_err());
        let mut empty = Session::new();
        assert!(empty.execute("layer.parallelLayer", json!({})).is_err());
    }
}
