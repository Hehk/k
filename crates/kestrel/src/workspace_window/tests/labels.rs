use super::*;

#[gpui::test]
fn control_characters_in_repository_names_render_on_open_and_restore(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let info = repository(&dir.path().join("repo\nname\r\t\u{7}\u{85}\u{2028}\u{2029}"));
    let title = r"repo\nname\r\t\u{7}\u{85}\u{2028}\u{2029}";
    let selector = r"tab-title-repo\nname\r\t\u{7}\u{85}\u{2028}\u{2029}";
    let store_path = dir.path().join("state.json");
    let workspace = shared(cx, store_path.clone());
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    handle
        .update(cx, |root, _, cx| open(root, info.clone(), cx))
        .unwrap();
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(cx);
    handle
        .read_with(cx, |root, cx| {
            assert_eq!(root.top_bar.read(cx).active_tab().unwrap().title, title);
        })
        .unwrap();
    assert!(cx.debug_bounds(selector).is_some());
    let state = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    let tab = state.windows()[0].active_tab.unwrap();
    let project = state
        .project(state.tab(tab).unwrap().route.project().unwrap())
        .unwrap();
    assert_eq!(project.directory, info.directory);
    assert_eq!(project.common_dir, info.common_dir);
    assert_eq!(
        Repository::open(&project.directory)
            .unwrap()
            .info()
            .unwrap(),
        info
    );
    cx.quit();
    assert_eq!(Store::new(store_path.clone()).load().unwrap().state, state);

    let restored = shared(cx, store_path);
    let handle = cx
        .cx
        .update(|cx| Kestrel::open(restored.clone(), state.windows()[0].id, cx).unwrap());
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(cx);
    assert!(cx.debug_bounds(selector).is_some());
    assert_eq!(
        restored.read_with(cx, |workspace, _| workspace.state().clone()),
        state
    );
    handle
        .read_with(cx, |root, cx| {
            assert_eq!(root.top_bar.read(cx).active_tab().unwrap().title, title);
        })
        .unwrap();
}
