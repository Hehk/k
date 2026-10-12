use kestrel::state::{AppState, Route, StateError, WindowId};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub enum Closed {
    Tab { index: usize, route: Route },
    Window { tabs: Vec<Route>, active: usize },
}

impl Closed {
    pub fn removed(before: &AppState, after: &AppState) -> Vec<Self> {
        let mut closed = Vec::new();
        for window in before.windows() {
            let removed = window
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, tab)| after.tab(tab.id).is_err())
                .collect::<Vec<_>>();
            if removed.is_empty() {
                continue;
            }
            if after.window(window.id).is_err() && removed.len() == window.tabs.len() {
                closed.push(Self::Window {
                    tabs: window.tabs.iter().map(|tab| tab.route.clone()).collect(),
                    active: window
                        .tabs
                        .iter()
                        .position(|tab| Some(tab.id) == window.active_tab)
                        .unwrap_or(0),
                });
            } else {
                closed.extend(removed.into_iter().map(|(index, tab)| Self::Tab {
                    index,
                    route: tab.route.clone(),
                }));
            }
        }
        closed
    }

    pub fn restore(
        &self,
        state: &mut AppState,
        target: Option<WindowId>,
    ) -> Result<WindowId, StateError> {
        match self {
            Self::Tab { index, route } => {
                let target = match target {
                    Some(target) => target,
                    None => state.new_window()?,
                };
                state.insert_tab(target, *index, route.clone())?;
                Ok(target)
            }
            Self::Window { tabs, active } => {
                let target = state.new_window()?;
                let mut selected = None;
                for (index, route) in tabs.iter().enumerate() {
                    let tab = state.insert_tab(target, index, route.clone())?;
                    if index == *active {
                        selected = Some(tab);
                    }
                }
                if let Some(tab) = selected {
                    state.select_tab(tab)?;
                }
                Ok(target)
            }
        }
    }
}
