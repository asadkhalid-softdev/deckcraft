//! Presentation references use stable slide/master/layout and object IDs, never screen coordinates.
use super::MAX_PROMPT;
use crate::{SlideApp, ViewMode};
use deckcraft_engine::{DocState, Selection, Session, Target};
use deckcraft_model::{LayoutId, MasterId, Presentation, Shape, SlideId};
use serde_json::{Value, json};

pub(super) const MAX_REFERENCES: usize = 8;

#[derive(Clone, Debug, PartialEq)]
enum Place {
    Slide(SlideId),
    Master(MasterId),
    Layout(MasterId, LayoutId),
    Handout,
    Notes,
}

#[derive(Clone, Debug)]
pub(super) struct Reference {
    pub id: u64,
    document: u64,
    place: Place,
    selection: Selection,
    pub text: String,
    snapshot: Value,
}

impl Reference {
    pub fn capture(session: &Session, id: u64) -> Result<Self, String> {
        let st = session.active().ok_or("Open a presentation first")?;
        let sel = st.selection.clone();
        let place = match sel.target {
            Target::Slides => Place::Slide(st.current_slide().ok_or("Select a slide first")?.id),
            Target::Master { master } => Place::Master(st.doc.masters.get(master).ok_or("Master no longer exists")?.id),
            Target::Layout { master, layout } => {
                let m = st.doc.masters.get(master).ok_or("Master no longer exists")?;
                Place::Layout(m.id, m.layouts.get(layout).ok_or("Layout no longer exists")?.id)
            }
            Target::Handout => Place::Handout,
            Target::NotesMaster => Place::Notes,
        };
        if sel.shapes.len() > 256 {
            return Err("Select at most 256 objects for one reference".into());
        }
        let mut bytes = 0usize;
        deckcraft_model::walk(st.shapes(), &mut |shape, _| {
            if (sel.shapes.is_empty() || sel.shapes.contains(&shape.id))
                && let Some(t) = &shape.text
            {
                for p in &t.paragraphs {
                    for r in &p.runs {
                        bytes = bytes.saturating_add(r.text.len());
                    }
                }
            }
        });
        if sel.text.as_ref().is_none_or(|t| !t.is_range()) && bytes > MAX_PROMPT {
            return Err("Selected context is too long; select fewer objects".into());
        }
        let (text, snapshot) = if let Some(t) = sel.text.as_ref().filter(|t| t.is_range()) {
            let body = deckcraft_engine::cmd::text::body_of(st, t).ok_or("Selected text is no longer available")?;
            let (a, b) = t.ordered();
            let text = deckcraft_model::edit::text_range(body, a, b);
            (text.clone(), json!({"kind":"text", "text":text}))
        } else {
            let selected: Vec<Value> = if sel.shapes.is_empty() {
                st.shapes().iter().take(256).map(|s| deckcraft_engine::cmd::inspect::shape_summary(None, s, 0)).collect()
            } else {
                sel.shapes
                    .iter()
                    .map(|id| {
                        st.shape(*id).map(|s| deckcraft_engine::cmd::inspect::shape_summary(None, s, 0)).ok_or("Selected object no longer exists")
                    })
                    .collect::<Result<_, _>>()?
            };
            let snapshot = json!({"kind":if sel.shapes.is_empty(){"slide"}else{"objects"}, "objects": selected});
            (snapshot.to_string(), snapshot)
        };
        if text.len() > MAX_PROMPT {
            return Err("Selected context exceeds 32,000 bytes. Select fewer objects or less text.".into());
        }
        Ok(Self { id, document: st.uid, place, selection: sel, text, snapshot })
    }
    pub fn same_selection(&self, other: &Self) -> bool {
        self.document == other.document && self.place == other.place && self.selection == other.selection && self.text == other.text
    }
    pub fn label(&self) -> String {
        let kind = if self.selection.text.as_ref().is_some_and(|t| t.is_range()) {
            "Selected text"
        } else if !self.selection.shapes.is_empty() {
            "Selected objects"
        } else {
            "Slide"
        };
        format!("{kind} {}", self.id)
    }
    fn selection(&self, st: &DocState) -> Result<Selection, String> {
        if st.uid != self.document {
            return Err("Reference belongs to another presentation".into());
        }
        let mut sel = self.selection.clone();
        let missing = "Referenced slide, master or layout was removed";
        sel.target = match self.place {
            Place::Slide(id) => {
                sel.slide = st.doc.slides.iter().position(|s| s.id == id).ok_or(missing)?;
                Target::Slides
            }
            Place::Master(id) => Target::Master { master: st.doc.masters.iter().position(|m| m.id == id).ok_or(missing)? },
            Place::Layout(mid, lid) => {
                let master = st.doc.masters.iter().position(|m| m.id == mid).ok_or(missing)?;
                let layout = st.doc.masters.get(master).and_then(|m| m.layouts.iter().position(|l| l.id == lid)).ok_or(missing)?;
                Target::Layout { master, layout }
            }
            Place::Handout => Target::Handout,
            Place::Notes => Target::NotesMaster,
        };
        for id in &sel.shapes {
            if deckcraft_model::find_shape(shapes(&st.doc, &sel), *id).is_none() {
                return Err("Referenced object was removed".into());
            }
        }
        Ok(sel)
    }
    pub fn validate(&self, session: &Session) -> Result<(), String> {
        self.selection(session.active().ok_or("Open the referenced presentation first")?).map(|_| ())
    }
    pub fn reveal(&self, app: &mut SlideApp) -> Result<(), String> {
        let sel = self.selection(app.session.active().ok_or("Open the referenced presentation first")?)?;
        app.ui.view = ViewMode::Normal;
        match sel.target {
            Target::Slides => {
                app.run("view.closeMaster", json!({}))?;
                app.run("slide.go", json!({"index":sel.slide}))?;
            }
            Target::Master { master } => {
                app.run("view.slideMaster", json!({"master":master}))?;
            }
            Target::Layout { master, layout } => {
                app.run("view.slideMaster", json!({"master":master,"layout":layout}))?;
            }
            Target::Handout => {
                app.run("view.handoutMaster", json!({}))?;
            }
            Target::NotesMaster => {
                app.run("view.notesMaster", json!({}))?;
            }
        }
        if !sel.shapes.is_empty() {
            if app.session.active().is_some_and(|d| sel.shapes.iter().any(|id| d.shape(*id).is_some_and(|s| s.hidden))) {
                return Err("A referenced object is hidden. Show it before following the reference.".into());
            }
            app.run("selection.select", json!({"ids":sel.shapes}))?;
        }
        if let Some(t) = sel.text.as_ref().filter(|t| t.is_range()) {
            app.run("text.edit", json!({"id":t.shape,"notes":t.notes,"cell":t.cell}))?;
            // Text offsets are restored only if they still name the captured excerpt.
            let current = app.session.active().and_then(|st| deckcraft_engine::cmd::text::body_of(st, t)).map(|b| {
                let (a, z) = t.ordered();
                deckcraft_model::edit::text_range(b, a, z)
            });
            if current.as_deref() == Some(self.text.as_str()) {
                app.run("text.select", json!({"anchor":t.anchor,"caret":t.caret}))?;
            } else {
                app.set_status("Referenced text changed; its containing object is selected.");
                app.run("text.exit", json!({}))?;
            }
        }
        Ok(())
    }
    pub fn context(&self) -> Value {
        json!({"reference":self.label(),"document":self.document,"selection":self.selection,"snapshot":self.snapshot,"text":self.text})
    }
}

fn shapes<'a>(doc: &'a Presentation, sel: &Selection) -> &'a [Shape] {
    match sel.target {
        Target::Slides => doc.slides.get(sel.slide).map(|s| s.shapes.as_slice()).unwrap_or(&[]),
        Target::Master { master } => doc.masters.get(master).map(|m| m.shapes.as_slice()).unwrap_or(&[]),
        Target::Layout { master, layout } => doc.masters.get(master).and_then(|m| m.layouts.get(layout)).map(|l| l.shapes.as_slice()).unwrap_or(&[]),
        t => deckcraft_engine::page_master(doc, t).map(|m| m.shapes.as_slice()).unwrap_or(&[]),
    }
}
