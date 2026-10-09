use std::time::Duration;

pub const TITLEBAR_HEIGHT: f32 = 40.;
pub const SIDE_WIDTH: f32 = 88.;
pub const TAB_HEIGHT: f32 = 26.;
pub const PILL_HEIGHT: f32 = 30.;
pub const PILL_PADDING: f32 = 2.;
pub const TAB_GAP: f32 = 2.;
const MIN_TAB_WIDTH: f32 = 100.;
const ANIMATION_DURATION: Duration = Duration::from_millis(160);

pub type TabId = u64;

#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragToken {
    pub source: u64,
    pub serial: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Idle,
    Dragging { tab: TabId, token: DragToken },
    Detaching { tab: TabId, request: DragToken },
    Closing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Geometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub scroll: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub pill_width: f32,
    pub tab_width: f32,
    pub scroll: f32,
    geometry: Geometry,
    count: usize,
}

impl Layout {
    fn new(geometry: Geometry, count: usize) -> Self {
        let pill_width = (geometry.width - SIDE_WIDTH * 2.).max(0.);
        let gaps = count.saturating_sub(1) as f32 * TAB_GAP;
        let tab_width =
            ((pill_width - PILL_PADDING * 2. - gaps) / count.max(1) as f32).max(MIN_TAB_WIDTH);
        let content_width = count as f32 * tab_width + gaps + PILL_PADDING * 2.;
        let scroll = geometry
            .scroll
            .clamp(-(content_width - pill_width).max(0.), 0.);
        Self {
            pill_width,
            tab_width,
            scroll,
            geometry,
            count,
        }
    }

    pub fn slot_x(&self, index: usize) -> f32 {
        PILL_PADDING + index as f32 * (self.tab_width + TAB_GAP)
    }

    pub fn slot_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.count < 2 {
            return None;
        }
        let x = x - self.geometry.x - SIDE_WIDTH;
        let y = y - self.geometry.y;
        let top = (TITLEBAR_HEIGHT - TAB_HEIGHT) / 2.;
        if x < PILL_PADDING
            || x >= self.pill_width - PILL_PADDING
            || y < top
            || y >= top + TAB_HEIGHT
        {
            return None;
        }
        let content_x = x - self.scroll - PILL_PADDING;
        if content_x < 0. {
            return None;
        }
        let pitch = self.tab_width + TAB_GAP;
        let index = (content_x / pitch).floor() as usize;
        (index < self.count && content_x - index as f32 * pitch < self.tab_width).then_some(index)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    Add,
    Select(TabId),
    SelectIndex(usize),
    SelectNext,
    SelectPrevious,
    Close(TabId),
    CloseActive,
    Hovered { tab: TabId, hovered: bool },
    DragStarted { tab: TabId, token: DragToken },
    DragMoved { token: DragToken, x: f32, y: f32 },
    Released { token: DragToken, outside: bool },
    Cancelled { token: DragToken },
    LayoutChanged(Geometry),
    Frame { generation: u64, elapsed: Duration },
    DetachCompleted { request: DragToken, success: bool },
    BackgroundPressed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WindowCommand {
    OpenDetached {
        request: DragToken,
        tab: Tab,
        next_tab_id: TabId,
    },
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Reveal(TabId),
    RequestFrame { generation: u64, elapsed: Duration },
    Window(WindowCommand),
    BeginWindowDrag,
    FocusDrag,
    RestoreFocus,
    StopDrag,
}

pub struct TabView<'a> {
    pub tab: &'a Tab,
    pub active: bool,
    pub hidden: bool,
    pub close_visible: bool,
    pub offset: f32,
    pub opacity: f32,
}

pub struct ViewState<'a> {
    pub tabs: Vec<TabView<'a>>,
    pub layout: Layout,
    pub interactive: bool,
    pub draggable: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Sample {
    id: TabId,
    x: f32,
    opacity: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct Animation {
    generation: u64,
    elapsed: Duration,
    from: Vec<Sample>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    tabs: Vec<Tab>,
    active: Option<TabId>,
    hovered: Option<TabId>,
    next_tab_id: TabId,
    next_drag: DragToken,
    phase: Phase,
    geometry: Geometry,
    generation: u64,
    animation: Option<Animation>,
}

impl Model {
    pub fn new(source: u64) -> Self {
        Self::with_tab(
            source,
            Tab {
                id: 1,
                title: "Project 1".into(),
            },
            2,
        )
    }

    pub fn with_tab(source: u64, tab: Tab, next_tab_id: TabId) -> Self {
        Self {
            active: Some(tab.id),
            hovered: None,
            next_tab_id: next_tab_id.max(tab.id + 1),
            tabs: vec![tab],
            next_drag: DragToken { source, serial: 1 },
            phase: Phase::Idle,
            geometry: Geometry::default(),
            generation: 0,
            animation: None,
        }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|tab| Some(tab.id) == self.active)
    }

    pub fn next_drag(&self) -> DragToken {
        self.next_drag
    }

    pub fn drag_token(&self) -> Option<DragToken> {
        match self.phase {
            Phase::Dragging { token, .. } => Some(token),
            _ => None,
        }
    }

    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    pub fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    pub fn view(&self) -> ViewState<'_> {
        let layout = Layout::new(self.geometry, self.tabs.len());
        let progress = self.animation.as_ref().map_or(1., |animation| {
            let t = (animation.elapsed.as_secs_f32() / ANIMATION_DURATION.as_secs_f32()).min(1.);
            t * t * (3. - 2. * t)
        });
        let tabs =
            self.tabs
                .iter()
                .enumerate()
                .map(|(index, tab)| {
                    let active = Some(tab.id) == self.active;
                    let target_opacity = if active { 1. } else { 0.88 };
                    let from = self.animation.as_ref().and_then(|animation| {
                        animation.from.iter().find(|sample| sample.id == tab.id)
                    });
                    let (offset, opacity) = from.map_or((0., target_opacity), |sample| {
                        (
                            (sample.x - layout.slot_x(index)) * (1. - progress),
                            sample.opacity + (target_opacity - sample.opacity) * progress,
                        )
                    });
                    let hidden = match self.phase {
                        Phase::Dragging { tab: source, .. }
                        | Phase::Detaching { tab: source, .. } => source == tab.id,
                        _ => false,
                    };
                    TabView {
                        tab,
                        active,
                        hidden,
                        close_visible: self.phase == Phase::Idle
                            && self.tabs.len() > 1
                            && (active || self.hovered == Some(tab.id)),
                        offset,
                        opacity,
                    }
                })
                .collect();
        let interactive = self.phase == Phase::Idle;
        ViewState {
            tabs,
            layout,
            interactive,
            draggable: interactive && self.tabs.len() > 1,
        }
    }

    fn samples(&self) -> Vec<Sample> {
        let view = self.view();
        view.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| Sample {
                id: tab.tab.id,
                x: view.layout.slot_x(index) + tab.offset,
                opacity: tab.opacity,
            })
            .collect()
    }

    fn animate(&mut self, from: Vec<Sample>, commands: &mut Vec<Command>) {
        self.generation += 1;
        self.animation = Some(Animation {
            generation: self.generation,
            elapsed: Duration::ZERO,
            from,
        });
        commands.push(Command::RequestFrame {
            generation: self.generation,
            elapsed: Duration::ZERO,
        });
    }

    fn select_tab(&mut self, id: TabId, commands: &mut Vec<Command>) {
        if self.tab_index(id).is_none() {
            return;
        }
        commands.push(Command::Reveal(id));
        if self.active != Some(id) {
            let from = self.samples();
            self.active = Some(id);
            self.animate(from, commands);
        }
    }

    fn remove_tab(&mut self, id: TabId, commands: &mut Vec<Command>) {
        let Some(index) = self.tab_index(id) else {
            return;
        };
        let from = self.samples();
        self.tabs.remove(index);
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        if self.tabs.is_empty() {
            self.active = None;
            self.phase = Phase::Closing;
            self.animation = None;
            commands.push(Command::Window(WindowCommand::Close));
        } else {
            if self.active == Some(id) {
                self.active = Some(self.tabs[index.min(self.tabs.len() - 1)].id);
            }
            commands.push(Command::Reveal(self.active.unwrap()));
            self.animate(from, commands);
        }
    }

    pub fn update(&mut self, message: Msg) -> Vec<Command> {
        let mut commands = Vec::new();
        match message {
            Msg::LayoutChanged(geometry) if self.phase != Phase::Closing => {
                if self.geometry != geometry {
                    let from = self.samples();
                    let resized = self.geometry.width != geometry.width;
                    if resized
                        && self.geometry.width > 0.
                        && self.phase == Phase::Idle
                        && let Some(active) = self.active
                    {
                        commands.push(Command::Reveal(active));
                    }
                    self.geometry = geometry;
                    if resized && self.animation.is_some() {
                        self.animate(from, &mut commands);
                    }
                }
            }
            Msg::Frame {
                generation,
                elapsed,
            } => {
                if let Some(animation) = &mut self.animation
                    && animation.generation == generation
                    && elapsed > animation.elapsed
                {
                    if elapsed >= ANIMATION_DURATION {
                        self.animation = None;
                    } else {
                        animation.elapsed = elapsed;
                        commands.push(Command::RequestFrame {
                            generation,
                            elapsed,
                        });
                    }
                }
            }
            Msg::Add if self.phase == Phase::Idle => {
                let from = self.samples();
                let id = self.next_tab_id;
                self.next_tab_id += 1;
                self.tabs.push(Tab {
                    id,
                    title: format!("Project {id}"),
                });
                self.active = Some(id);
                commands.push(Command::Reveal(id));
                self.animate(from, &mut commands);
            }
            Msg::Select(id) if self.phase == Phase::Idle => {
                self.select_tab(id, &mut commands);
            }
            Msg::SelectIndex(index) if self.phase == Phase::Idle => {
                if let Some(tab) = self.tabs.get(index) {
                    self.select_tab(tab.id, &mut commands);
                }
            }
            Msg::SelectNext | Msg::SelectPrevious if self.phase == Phase::Idle => {
                let index = self.tab_index(self.active.unwrap()).unwrap();
                let count = self.tabs.len();
                let next = if message == Msg::SelectNext {
                    (index + 1) % count
                } else {
                    (index + count - 1) % count
                };
                self.select_tab(self.tabs[next].id, &mut commands);
            }
            Msg::CloseActive if self.phase == Phase::Idle => {
                self.remove_tab(self.active.unwrap(), &mut commands);
            }
            Msg::Close(id) if self.phase == Phase::Idle => {
                self.remove_tab(id, &mut commands);
            }
            Msg::Hovered { tab, hovered }
                if self.phase == Phase::Idle && self.tab_index(tab).is_some() =>
            {
                if hovered {
                    self.hovered = Some(tab);
                } else if self.hovered == Some(tab) {
                    self.hovered = None;
                }
            }
            Msg::BackgroundPressed if self.phase == Phase::Idle => {
                commands.push(Command::BeginWindowDrag);
            }
            Msg::DragStarted { tab, token }
                if self.phase == Phase::Idle
                    && token == self.next_drag
                    && self.tab_index(tab).is_some() =>
            {
                let from = self.samples();
                self.next_drag.serial += 1;
                self.phase = Phase::Dragging { tab, token };
                self.hovered = None;
                self.active = Some(tab);
                commands.push(Command::FocusDrag);
                self.animate(from, &mut commands);
            }
            Msg::DragMoved { token, x, y } => {
                if let Phase::Dragging {
                    tab,
                    token: current,
                } = self.phase
                    && token == current
                    && let Some(to) = self.view().layout.slot_at(x, y)
                    && let Some(from) = self.tab_index(tab)
                    && from != to
                {
                    let samples = self.samples();
                    let tab = self.tabs.remove(from);
                    self.tabs.insert(to, tab);
                    self.animate(samples, &mut commands);
                }
            }
            Msg::Released { token, outside } => {
                if let Phase::Dragging {
                    tab,
                    token: current,
                } = self.phase
                    && token == current
                {
                    commands.push(Command::RestoreFocus);
                    if outside {
                        self.phase = Phase::Detaching {
                            tab,
                            request: token,
                        };
                        commands.push(Command::Window(WindowCommand::OpenDetached {
                            request: token,
                            tab: self.tabs[self.tab_index(tab).unwrap()].clone(),
                            next_tab_id: self.next_tab_id,
                        }));
                    } else {
                        self.phase = Phase::Idle;
                    }
                }
            }
            Msg::Cancelled { token } if self.drag_token() == Some(token) => {
                self.phase = Phase::Idle;
                commands.push(Command::StopDrag);
                commands.push(Command::RestoreFocus);
            }
            Msg::DetachCompleted { request, success } => {
                if let Phase::Detaching {
                    tab,
                    request: pending,
                } = self.phase
                    && request == pending
                {
                    self.phase = Phase::Idle;
                    if success {
                        self.remove_tab(tab, &mut commands);
                    }
                }
            }
            _ => {}
        }
        commands
    }
}

#[cfg(test)]
mod tests;
