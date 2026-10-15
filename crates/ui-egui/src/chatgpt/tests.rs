use super::*;
use deckcraft_engine::Session;
use deckcraft_model::ShapeId;

fn app() -> SlideApp {
    SlideApp::new(Session::with_new(), Default::default())
}
fn run(a: &mut SlideApp, id: &str, p: Value) -> Value {
    execute_tool(a, "deckcraft_execute", &json!({"command":id,"params":p})).unwrap()
}
fn shape(a: &mut SlideApp, text: &str, rect: [f64; 4]) -> ShapeId {
    ShapeId(run(a, "shape.insert", json!({"preset":"rect","rect":rect,"text":text}))["id"].as_u64().unwrap() as u32)
}

#[test]
fn every_engine_feature_is_discoverable_and_auth_and_shell_are_excluded() {
    let mut a = app();
    let mut ids = Vec::new();
    let mut offset = 0;
    loop {
        let result = execute_tool(&mut a, "deckcraft_commands", &json!({"limit":100,"offset":offset})).unwrap();
        ids.extend(result["items"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap().to_owned()));
        if result["hasMore"] == false {
            break;
        }
        offset += 100;
    }
    for c in a.session.commands() {
        assert!(ids.iter().any(|id| id == c.id), "{} missing", c.id);
    }
    for feature in
        ["shape.move", "shape.resize", "arrange.group", "animation.add", "transition.set", "insert.table", "insert.chart", "view.slideMaster"]
    {
        assert!(ids.iter().any(|id| id == feature));
    }
    for id in ["ui.chatgpt.login", "app.openDialog", "app.dialog", "powershell", "cmd.exe"] {
        assert!(execute_tool(&mut a, "deckcraft_execute", &json!({"command":id})).is_err());
    }
    assert_eq!(tool_definitions().as_array().unwrap().len(), 7);
}

#[test]
fn assistant_builds_positions_formats_groups_and_undoes_real_objects() {
    let mut a = app();
    let id = shape(&mut a, "Launch plan", [40., 50., 240., 60.]);
    run(&mut a, "shape.move", json!({"ids":[id],"x":100.,"y":80.}));
    run(&mut a, "shape.resize", json!({"ids":[id],"w":300.,"h":90.}));
    run(&mut a, "selection.select", json!({"ids":[id]}));
    run(&mut a, "format.font", json!({"family":"Liberation Sans"}));
    run(&mut a, "format.bold", json!({"value":true}));
    let inspect = execute_tool(&mut a, "deckcraft_inspect", &json!({"scope":"shape","id":id})).unwrap();
    assert_eq!(inspect["box"]["x"], 100.0);
    assert_eq!(inspect["box"]["w"], 300.0);
    assert!(inspect.to_string().contains("Launch plan"));
    run(&mut a, "transition.set", json!({"kind":"fade","duration":600}));
    run(&mut a, "animation.add", json!({"effect":"fade","class":"entrance","ids":[id]}));
    let second = shape(&mut a, "Delivery", [440., 80., 180., 90.]);
    run(&mut a, "arrange.group", json!({"ids":[id,second]}));
    assert!(a.session.active().unwrap().shapes().iter().any(|s| matches!(s.kind, deckcraft_model::ShapeKind::Group { .. })));
    run(&mut a, "edit.undo", json!({}));
    assert!(a.session.active().unwrap().shape(id).is_some());
    assert!(a.session.active().unwrap().shape(second).is_some());
    let slide = execute_tool(&mut a, "deckcraft_inspect", &json!({"scope":"slide"})).unwrap();
    assert_eq!(slide["transition"]["kind"], "fade");
    assert_eq!(slide["animations"].as_array().unwrap().len(), 1);
}

#[test]
fn batches_report_partial_completion_and_headless_commands_never_open_dialogs() {
    let mut a = app();
    let count = a.session.active().unwrap().shapes().len();
    let result=execute_tool(&mut a,"deckcraft_batch",&json!({"commands":[{"command":"shape.insert","params":{"preset":"rect","rect":[20,20,100,50]}},{"command":"missing.command"},{"command":"slide.delete"}]})).unwrap();
    assert_eq!(result["results"].as_array().unwrap().len(), 1);
    assert_eq!(result["stoppedAt"], 1);
    assert_eq!(a.session.active().unwrap().shapes().len(), count + 1);
    assert!(execute_tool(&mut a, "deckcraft_execute", &json!({"command":"design.fonts"})).is_err());
    assert!(a.dialog.is_none());
    assert!(a.session.ui_requests.is_empty());
    assert!(execute_tool(&mut a, "deckcraft_execute", &json!({"command":"file.close"})).is_err());
    assert!(a.session.active().is_some());
    assert!(execute_tool(&mut a, "deckcraft_batch", &json!({"commands":vec![json!({"command":"slide.new"});33]})).is_err());
    assert!(execute_tool(&mut a, "deckcraft_interact", &json!({"method":"key","params":{"key":"W","cmd":true}})).is_err());
    assert!(execute_tool(&mut a, "deckcraft_interact", &json!({"method":"pointer","params":{"events":[{"kind":"down","x":1e100,"y":2}]}})).is_err());
}

#[test]
fn stable_object_references_follow_slide_reordering_and_do_not_change_content() {
    let mut a = app();
    let id = shape(&mut a, "Source", [50., 80., 200., 60.]);
    run(&mut a, "selection.select", json!({"ids":[id]}));
    let uid = document_id(&a);
    let revision = a.session.active().unwrap().revision;
    a.run("ui.chatgpt.attachSelection", json!({})).unwrap();
    a.run("ui.chatgpt.attachSelection", json!({})).unwrap();
    assert_eq!(a.chatgpt.references.len(), 1);
    assert!(a.chatgpt.open);
    assert_eq!(a.session.active().unwrap().revision, revision);
    assert!(!a.chatgpt.references[0].label().contains("Source"));
    run(&mut a, "slide.new", json!({"title":"Second slide"}));
    run(&mut a, "slide.move", json!({"from":0,"to":1}));
    a.run("ui.chatgpt.showReference", json!({"id":1})).unwrap();
    assert_eq!(document_id(&a), uid);
    assert_eq!(a.session.active().unwrap().selection.slide, 1);
    assert_eq!(a.session.active().unwrap().selection.shapes, vec![id]);
    a.run("ui.chatgpt.removeReference", json!({"id":1})).unwrap();
    assert!(a.chatgpt.references.is_empty());
    assert!(a.run("ui.chatgpt.showReference", json!({"id":u64::MAX})).is_err());
}

#[test]
fn references_capture_text_and_clear_on_document_switch() {
    let mut a = app();
    let id = shape(&mut a, "Hello café world", [50., 50., 240., 80.]);
    run(&mut a, "text.edit", json!({"id":id}));
    run(&mut a, "text.select", json!({"anchor":[0,6],"caret":[0,10]}));
    a.run("ui.chatgpt.attachSelection", json!({})).unwrap();
    assert_eq!(a.chatgpt.references[0].text, "café");
    run(&mut a, "text.exit", json!({}));
    a.run("ui.chatgpt.showReference", json!({"id":1})).unwrap();
    assert_eq!(a.session.active().unwrap().selection.text.as_ref().unwrap().anchor, (0, 6));
    a.session.execute("file.new", &json!({})).unwrap();
    poll(&mut a, &egui::Context::default());
    assert!(a.chatgpt.references.is_empty() && a.chatgpt.messages.is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn send_attaches_hidden_context_and_search_preference_without_changing_the_slide() {
    let mut a = app();
    let id = shape(&mut a, "User's private selected text", [40., 40., 240., 80.]);
    run(&mut a, "selection.select", json!({"ids":[id]}));
    a.run("ui.chatgpt.attachSelection", json!({})).unwrap();
    let (client, outgoing) = desktop::Client::test_pair();
    a.chatgpt.client = Some(client);
    a.chatgpt.ready = true;
    a.chatgpt.account = Some("test@example.com".into());
    a.chatgpt.thread = Some("thread".into());
    assert!(a.ui.chatgpt_web_search);
    a.run("ui.chatgpt.webSearch", json!({"value":false})).unwrap();
    a.run("ui.chatgpt.send", json!({"text":"Improve this layout"})).unwrap();
    let request = outgoing.try_recv().unwrap();
    assert_eq!(request["params"]["webSearch"], false);
    let prompt = request.pointer("/params/input/0/text").unwrap().as_str().unwrap();
    assert!(prompt.contains("User's private selected text"));
    assert_eq!(a.chatgpt.messages.last().unwrap().text, "Improve this layout");
    assert_eq!(a.chatgpt.messages.last().unwrap().references.len(), 1);
    assert!(a.chatgpt.references.is_empty());
    let preferences = serde_json::to_string(&a.ui).unwrap();
    assert!(!preferences.contains("private selected"));
    let loaded: crate::UiState = serde_json::from_str(&preferences).unwrap();
    assert!(!loaded.chatgpt_web_search);
    let legacy: crate::UiState = serde_json::from_str("{}").unwrap();
    assert!(legacy.chatgpt_web_search);
}

#[test]
fn stale_tool_calls_and_removed_objects_cannot_change_the_deck() {
    let mut a = app();
    let id = shape(&mut a, "Keep", [40., 40., 80., 40.]);
    run(&mut a, "selection.select", json!({"ids":[id]}));
    a.run("ui.chatgpt.attachSelection", json!({})).unwrap();
    a.chatgpt.busy = true;
    a.chatgpt.thread = Some("thread".into());
    a.chatgpt.turn = Some("turn".into());
    let call = json!({"threadId":"thread","turnId":"stale","tool":"deckcraft_execute","arguments":{"command":"edit.delete","params":{"ids":[id]}}});
    assert!(tool_call(&mut a, &call).is_err());
    assert!(a.session.active().unwrap().shape(id).is_some());
    a.chatgpt.clear();
    assert!(tool_call(&mut a, &call).is_err());
    assert!(commands(&a).iter().all(|c| c["id"] != "ui.chatgpt.stop" && c["id"] != "ui.chatgpt.disconnect"));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn file_reads_require_user_paths_and_run_without_touching_the_live_deck() {
    let mut a = app();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("research notes.txt");
    std::fs::write(&path, "Research café\nSource notes").unwrap();
    let args = json!({"path":path});
    assert!(execute_tool(&mut a, "deckcraft_read_file", &args).is_err());
    a.chatgpt.user_requests.push(format!("Use `{}`", path.display()));
    let revision = a.session.active().unwrap().revision;
    let uid = document_id(&a);
    let authorization = execute_tool(&mut a, "deckcraft_read_file", &args).unwrap();
    let result = files::read(&authorization, &args, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    assert_eq!(result["text"], "Research café\nSource notes");
    assert_eq!(document_id(&a), uid);
    assert_eq!(a.session.active().unwrap().revision, revision);
    a.chatgpt.clear();
    assert!(execute_tool(&mut a, "deckcraft_read_file", &args).is_err());
}

#[test]
fn render_tool_returns_real_slide_pixels_and_does_not_mutate_the_document() {
    let mut a = app();
    shape(&mut a, "Preview", [40., 40., 240., 80.]);
    let revision = a.session.active().unwrap().revision;
    let image = execute_tool(&mut a, "deckcraft_render", &json!({})).unwrap();
    let url = image["image_url"].as_str().unwrap();
    let encoded = url.strip_prefix("data:image/png;base64,").unwrap();
    let bytes = deckcraft_engine::cmd::base64_decode(encoded).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(image["width"].as_u64().unwrap() <= 800);
    assert_eq!(a.session.active().unwrap().revision, revision);
    assert!(execute_tool(&mut a, "deckcraft_render", &json!({"index":u64::MAX})).is_err());
}

#[test]
fn chat_typing_and_reference_removal_leave_slide_text_unchanged() {
    use egui_kittest::kittest::Queryable;
    let mut a = app();
    let id = shape(&mut a, "Keep this slide text", [40., 40., 260., 80.]);
    a.chatgpt.ready = true;
    a.chatgpt.account = Some("test@example.com".into());
    a.run("ui.chatgpt.attachSelection", json!({})).unwrap();
    let label = a.chatgpt.references[0].label();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440., 1000.)).build_ui_state(
        |ui, a: &mut SlideApp| {
            let ctx = ui.ctx().clone();
            a.logic(&ctx);
            a.ui(ui);
        },
        a,
    );
    h.run_steps(5);
    h.get_by_label("Message ChatGPT").focus();
    h.run_steps(2);
    h.event(egui::Event::Text("Please improve the layout".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().chatgpt.draft.starts_with("Please improve"));
    h.get_by_label(&label).click();
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().selection.shapes, vec![id]);
    h.get_by_label("Remove reference").click();
    h.run_steps(2);
    assert!(h.state().chatgpt.references.is_empty());
    assert_eq!(h.state().session.active().unwrap().shape(id).unwrap().text.as_ref().unwrap().text(), "Keep this slide text");
}

#[test]
fn right_click_menu_attaches_the_selected_object() {
    use egui_kittest::kittest::Queryable;
    let mut a = app();
    let id = shape(&mut a, "Context", [200., 160., 160., 90.]);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440., 1000.)).build_ui_state(
        |ui, a: &mut SlideApp| {
            let ctx = ui.ctx().clone();
            a.logic(&ctx);
            a.ui(ui);
        },
        a,
    );
    h.run_steps(5);
    let rect = h.state().slide_rect.unwrap();
    let scale = h.state().canvas_scale;
    let at = rect.min + egui::vec2(280. * scale, 205. * scale);
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.run_steps(1);
    }
    h.run_steps(2);
    h.get_all_by_label("Add to Chat").last().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().chatgpt.references.len(), 1);
    assert!(h.state().chatgpt.open);
    assert!(h.state().session.active().unwrap().shape(id).is_some());
}

#[test]
#[ignore = "manual native GPU preview with synthetic content; no account or network"]
fn presentation_chat_preview() {
    use egui_kittest::kittest::Queryable;
    // Use the always-embedded, openly licensed face rather than an OS substitution.
    deckcraft_fonts::FontDb::global().set_system_fallback(false);
    let mut a = app();
    run(&mut a, "design.fonts", json!({"major":"Ubuntu","minor":"Ubuntu"}));
    run(&mut a, "edit.selectAll", json!({}));
    run(&mut a, "edit.delete", json!({}));
    shape(&mut a, "A clearer launch plan", [60., 42., 780., 70.]);
    run(&mut a, "format.size", json!({"size":36}));
    shape(&mut a, "Research", [75., 190., 220., 160.]);
    shape(&mut a, "Build", [370., 190., 220., 160.]);
    let id = shape(&mut a, "Launch", [665., 190., 220., 160.]);
    a.run("ui.chatgpt.attachSelection", json!({})).unwrap();
    let label = a.chatgpt.references[0].label();
    a.chatgpt.ready = true;
    a.chatgpt.account = Some("Preview account".into());
    a.chatgpt.draft = "Align these cards and make the spacing consistent".into();
    a.chatgpt.push("ChatGPT","## Layout review\n\nThe three stages use **consistent spacing**.\n\n| Stage | Purpose |\n|---|---|\n| Research | Understand the audience |\n| Build | Develop the message |\n| Launch | Share the story |\n\n- Align the card tops\n- Keep text readable\n\n[Source](https://example.com)".into(),None);
    a.integrated_titlebar = true;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600., 1000.)).with_pixels_per_point(1.0).wgpu().build_ui_state(
        |ui, a: &mut SlideApp| {
            let ctx = ui.ctx().clone();
            a.logic(&ctx);
            a.ui(ui);
        },
        a,
    );
    h.input_mut().max_texture_side = Some(8192);
    h.run_steps(8);
    h.get_by_label(&label).hover();
    h.run_steps(2);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    h.render().unwrap().save(dir.join("chatgpt-deck-preview.png")).unwrap();
    h.state_mut().run("ui.chatgpt.settings", json!({})).unwrap();
    h.run_steps(3);
    h.render().unwrap().save(dir.join("chatgpt-deck-settings.png")).unwrap();
    assert!(h.state().session.active().unwrap().shape(id).is_some());
}
