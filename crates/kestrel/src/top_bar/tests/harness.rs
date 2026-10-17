use gpui::{Entity, FocusHandle, Subscription};

use super::*;

pub struct Harness {
    pub top_bar: Entity<TopBar>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Harness {
    pub fn new(initial: Option<(Tab, u64)>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let top_bar = cx.new(|cx| TopBar::new(initial, window, cx));
        let focus = cx.focus_handle();
        window.focus(&focus);
        let subscriptions = vec![
            cx.observe(&top_bar, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &top_bar,
                window,
                |_, bar, command, window, cx| match command {
                    WindowCommand::OpenDetached {
                        request,
                        tab,
                        next_tab_id,
                    } => {
                        let initial = Some((tab.clone(), *next_tab_id));
                        let success = cx
                            .open_window(crate::window_options(None), |window, cx| {
                                cx.new(|cx| Self::new(initial, window, cx))
                            })
                            .is_ok();
                        bar.update(cx, |bar, cx| {
                            bar.dispatch(
                                Msg::DetachCompleted {
                                    request: *request,
                                    success,
                                },
                                window,
                                cx,
                            )
                        });
                    }
                    WindowCommand::Close => window.remove_window(),
                    WindowCommand::NewTab
                    | WindowCommand::LayoutChanged { .. }
                    | WindowCommand::Back
                    | WindowCommand::Forward
                    | WindowCommand::RecentlyViewed => {}
                },
            ),
        ];
        Self {
            top_bar,
            focus,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .top_bar
            .read(cx)
            .active_tab()
            .map(|tab| tab.title.clone())
            .unwrap_or_default();
        div()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, action: &TabAction, window, cx| {
                this.top_bar
                    .update(cx, |bar, cx| bar.dispatch(action.message(), window, cx));
            }))
            .flex()
            .flex_col()
            .size_full()
            .child(self.top_bar.clone())
            .child(
                div()
                    .flex_1()
                    .debug_selector(move || format!("content-title-{title}")),
            )
    }
}
