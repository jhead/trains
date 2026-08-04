//! The window system — draggable, closable, position-remembering panels.
//!
//! Binding standard: [`docs/design/03-ui-system.md`](../../../docs/design/03-ui-system.md) §5.
//!
//! Rail Town's reference is a management sim (RollerCoaster Tycoon, Locomotion),
//! not a fixed HUD. Only three things are permanently on screen — the menu row,
//! the status strip and the network health strip. Everything else is a window
//! the player opened, and a window the player can move, stack and close.
//!
//! # Ownership
//!
//! A window is **one component on one panel root**: [`UiWindow`]. Everything
//! else — where it sits, whether it is open, what order it stacks in, and its
//! title bar — belongs to [`WindowManager`] and the systems here. A panel author
//! writes their content and nothing about layout.
//!
//! Position lives in the resource rather than on the entity, so a panel that
//! despawns and respawns its own body (the Goals panel does) comes back exactly
//! where the player left it.
//!
//! # `Esc`
//!
//! 03 §10.1: `Esc` unwinds **one** layer per press. The existing unwind order
//! lives across several systems in `Update` — the track tool cancels a drag,
//! selection clears, and the shell opens the pause menu from `PreUpdate`.
//! [`close_top_window_on_escape`] runs in `PreUpdate` ahead of all of them and,
//! when it closes a window, *consumes* the key press so nothing downstream also
//! fires. That is how a window can never trap the player and can never
//! accidentally pause the game on the way out.
//!
//! # Sound
//!
//! Design 10 §5 gives panel open and close a brief airy sweep. It is emitted
//! **once, here** ([`panel_cues`]), not by each panel: this resource is the one
//! place a panel's visibility actually flips, so a window that spawns, despawns
//! and respawns its own body cannot chatter, and a new window costs nothing.
//!
//! [`panel_cues`] records the first frame rather than announcing it, which is
//! what keeps [`WindowManager::new`]'s boot-open Town Talk silent: a panel that
//! was already up is not a panel the player just opened.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::audio::UiCue;
use crate::palette::{BALLAST_L, BG0, BG1, HI, OUTLINE};
use crate::ui::kit::{
    control_border, cursor_to_ui, micro_font, panel_node, screen_to_ui, text_accent,
    text_secondary, WorldClickBlocker, SPACE_1, SPACE_2, TITLE_BAR_H, TOP_CHROME_H,
};

/// Base `ZIndex` for windows. Above the world and the strips, below modals.
const WINDOW_Z_BASE: i32 = 100;

/// Every window in the game.
///
/// Adding a variant is the whole cost of adding a window: the menu row, the
/// manager and `Esc` all iterate [`WindowId::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowId {
    /// Full per-station service list — the health strip's detail view.
    Network,
    TownTalk,
    Ledger,
    Alerts,
    Goals,
    Neighbours,
    /// Every train the player owns, placed or still in the yard.
    Trains,
    /// Opened by selecting something in the world, not by a button.
    Inspector,
}

impl WindowId {
    pub const ALL: &'static [Self] = &[
        Self::Network,
        Self::TownTalk,
        Self::Ledger,
        Self::Alerts,
        Self::Goals,
        Self::Neighbours,
        Self::Trains,
        Self::Inspector,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Network => "Network",
            Self::TownTalk => "Town Talk",
            Self::Ledger => "Ledger",
            Self::Alerts => "Alerts",
            Self::Goals => "Goals",
            Self::Neighbours => "Neighbours",
            Self::Trains => "Trains",
            Self::Inspector => "Inspector",
        }
    }

    /// Where the window opens the first time, before the player moves it.
    ///
    /// Chosen so the centre of the screen stays clear (03 §5) and so two
    /// windows opened together do not land on top of each other.
    fn default_offset(self) -> Vec2 {
        let top = TOP_CHROME_H + SPACE_2;
        match self {
            Self::Network => Vec2::new(SPACE_2, top),
            Self::TownTalk => Vec2::new(SPACE_2, top + 120.0),
            Self::Ledger => Vec2::new(SPACE_2 + 160.0, top),
            Self::Alerts => Vec2::new(-(320.0 + SPACE_2), top),
            Self::Goals => Vec2::new(-(264.0 + SPACE_2), top + 96.0),
            Self::Neighbours => Vec2::new(-(320.0 + SPACE_2), top + 200.0),
            Self::Trains => Vec2::new(SPACE_2 + 340.0, top),
            Self::Inspector => Vec2::new(-(280.0 + SPACE_2), top),
        }
    }

    /// `true` when the offset is measured from the right edge.
    fn anchors_right(self) -> bool {
        self.default_offset().x < 0.0
    }
}

/// Per-window state. Position is in UI texels from the top-left of the screen.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowState {
    pub open: bool,
    pub pos: Vec2,
    /// `true` once the player has dragged it.
    ///
    /// Until then the window keeps tracking its default corner, so resizing the
    /// game window (or changing UI scale) does not strand a right-hand window
    /// off the edge of the screen. After a drag the position is the player's,
    /// and nothing moves it again.
    moved: bool,
}

/// A drag in progress.
#[derive(Debug, Clone, Copy)]
struct WindowDrag {
    id: WindowId,
    /// Cursor position, in UI texels, when the drag began.
    grab: Vec2,
    /// Window position when the drag began.
    origin: Vec2,
}

/// Open state, position and stacking order for every window.
///
/// `order` is back-to-front. The last entry is the top window, which is the one
/// `Esc` closes and the one a fresh `open` raises to.
#[derive(Resource, Debug)]
pub struct WindowManager {
    states: Vec<(WindowId, WindowState)>,
    order: Vec<WindowId>,
    drag: Option<WindowDrag>,
}

/// Deliberately equal to [`WindowManager::new`]: a manager with no slots would
/// panic on the first lookup, and `init_resource` is too easy to reach for.
impl Default for WindowManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowManager {
    /// Every window closed, except the one the game speaks through.
    ///
    /// DESIGN.md: *"The complaint feed is both the diagnostic layer and the
    /// emotional hook."* A hook the player has to find on a menu is not a hook,
    /// and the first-run nudge (`onboarding::nudge`) pushes the game's opening
    /// line straight into this feed — into a closed panel, where nobody read it.
    /// So **Town Talk, and only Town Talk, is up from the first frame**.
    ///
    /// It opens through [`Self::open`] rather than by setting the flag, so it
    /// joins the stacking order like any other window: `Esc` closes it, the
    /// close box closes it, and once closed it stays closed. Position is still
    /// the resource's, so a player who moves it and closes it finds it where
    /// they left it.
    pub fn new() -> Self {
        let mut manager = Self {
            states: WindowId::ALL
                .iter()
                .map(|id| (*id, WindowState::default()))
                .collect(),
            order: Vec::new(),
            drag: None,
        };
        manager.open(WindowId::TownTalk);
        manager
    }

    fn slot(&self, id: WindowId) -> &WindowState {
        self.states
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, s)| s)
            .expect("every WindowId has a slot")
    }

    fn slot_mut(&mut self, id: WindowId) -> &mut WindowState {
        self.states
            .iter_mut()
            .find(|(i, _)| *i == id)
            .map(|(_, s)| s)
            .expect("every WindowId has a slot")
    }

    pub fn is_open(&self, id: WindowId) -> bool {
        self.slot(id).open
    }

    /// The window `Esc` would close.
    pub fn top(&self) -> Option<WindowId> {
        self.order.last().copied()
    }

    /// Open `id` and raise it to the front. Already-open windows just raise.
    pub fn open(&mut self, id: WindowId) {
        self.slot_mut(id).open = true;
        self.raise(id);
    }

    pub fn close(&mut self, id: WindowId) {
        self.slot_mut(id).open = false;
        self.order.retain(|i| *i != id);
    }

    pub fn toggle(&mut self, id: WindowId) {
        if self.is_open(id) {
            self.close(id);
        } else {
            self.open(id);
        }
    }

    /// Bring an open window to the front of the stack.
    pub fn raise(&mut self, id: WindowId) {
        if !self.is_open(id) {
            return;
        }
        if self.order.last() == Some(&id) {
            return;
        }
        self.order.retain(|i| *i != id);
        self.order.push(id);
    }

    fn depth(&self, id: WindowId) -> i32 {
        self.order
            .iter()
            .position(|i| *i == id)
            .map(|p| p as i32)
            .unwrap_or(0)
    }

    /// Move a window because the player dragged it. Snapped to whole texels
    /// (03 §9: nothing the UI draws ever lands on a fractional coordinate).
    fn place(&mut self, id: WindowId, pos: Vec2) {
        let slot = self.slot_mut(id);
        slot.pos = Vec2::new(pos.x.round(), pos.y.round());
        slot.moved = true;
    }

    /// Park an un-dragged window at its default corner for this screen size.
    ///
    /// Callers must check [`Self::needs_parking`] first: reaching for
    /// `slot_mut` is what marks the resource changed, and a manager that marks
    /// itself changed every frame turns the layout pass into per-frame work.
    fn park(&mut self, id: WindowId, pos: Vec2) {
        let slot = self.slot_mut(id);
        slot.pos = Vec2::new(pos.x.round(), pos.y.round());
    }

    /// `Some(current position)` when `id` is still tracking its default corner.
    fn needs_parking(&self, id: WindowId) -> Option<Vec2> {
        let slot = self.slot(id);
        (!slot.moved).then_some(slot.pos)
    }
}

/// Marks a panel root as a window. The only thing a panel author writes.
#[derive(Component, Debug, Clone, Copy)]
pub struct UiWindow {
    pub id: WindowId,
}

impl UiWindow {
    pub fn new(id: WindowId) -> Self {
        Self { id }
    }
}

/// The draggable strip at the top of a window.
#[derive(Component, Debug, Clone, Copy)]
pub struct WindowTitleBar {
    pub id: WindowId,
}

/// The close box on a window's title bar.
#[derive(Component, Debug, Clone, Copy)]
pub struct WindowCloseButton {
    pub id: WindowId,
}

/// Set so a window's own `Esc` / hotkey handling can see that the key was taken.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowEscSet;

/// Bundle for a window root built inside `ui/`.
///
/// Foreign panels (Goals, Neighbours, Inspector) are adopted in
/// [`super::adapters`] instead; they keep their own root and only gain a
/// [`UiWindow`].
pub fn window_root(id: WindowId, width: f32) -> impl Bundle {
    let (node, bg, border) = panel_node(Node {
        position_type: PositionType::Absolute,
        width: Val::Px(width),
        max_height: Val::Percent(70.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(SPACE_1),
        padding: UiRect::all(Val::Px(SPACE_1)),
        display: Display::None,
        ..default()
    });
    (
        UiWindow::new(id),
        WorldClickBlocker,
        Interaction::default(),
        node,
        bg,
        border,
    )
}

/// Give every newly-seen window a title bar with a drag handle and a close box.
///
/// Runs on `Added<UiWindow>`, so it costs nothing on a normal frame and still
/// re-dresses a panel that despawned and respawned its own root.
pub fn dress_new_windows(
    mut commands: Commands,
    added: Query<(Entity, &UiWindow), Added<UiWindow>>,
) {
    for (entity, window) in &added {
        let id = window.id;
        let bar = commands
            .spawn((
                WindowTitleBar { id },
                Button,
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(TITLE_BAR_H),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    padding: UiRect::axes(Val::Px(SPACE_1), Val::Px(0.0)),
                    border_radius: BorderRadius::ZERO,
                    flex_shrink: 0.0,
                    ..default()
                },
                BackgroundColor(BG0),
            ))
            .with_children(|row| {
                row.spawn((Text::new(id.title()), micro_font(), text_accent()));
                let (node, bg, border) = super::kit::chrome_button_node(SPACE_1, 0.0);
                row.spawn((Button, WindowCloseButton { id }, node, bg, border))
                    .with_children(|b| {
                        row_close_glyph(b);
                    });
            })
            .id();
        commands.entity(entity).insert_children(0, &[bar]);
    }
}

fn row_close_glyph(parent: &mut ChildSpawnerCommands) {
    parent.spawn((Text::new("x"), micro_font(), text_secondary()));
}

/// Push open state, position and stacking order onto the actual nodes.
///
/// Gated on `WindowManager` change detection plus newly-added roots, so an idle
/// frame with no window interaction does no layout work at all.
pub fn apply_window_layout(
    mut manager: ResMut<WindowManager>,
    windows: Query<&Window, With<PrimaryWindow>>,
    ui_scale: Res<UiScale>,
    mut roots: Query<(&UiWindow, &mut Node, &mut ZIndex)>,
    added: Query<(), Added<UiWindow>>,
    mut last_screen: Local<Vec2>,
) {
    // `Val::Px` is UI-px, and `Window::width/height` are logical px — see
    // `kit::cursor_to_ui` for why these are not the same space.
    let screen = windows
        .single()
        .map(|w| screen_to_ui(Vec2::new(w.width(), w.height()), ui_scale.0))
        .unwrap_or(Vec2::new(1280.0, 720.0));
    let reflow = screen != *last_screen;
    if !manager.is_changed() && added.is_empty() && !reflow {
        return;
    }
    *last_screen = screen;

    // Windows the player has not moved keep tracking their default corner.
    for id in WindowId::ALL.iter().copied() {
        let Some(current) = manager.needs_parking(id) else {
            continue;
        };
        let offset = id.default_offset();
        let x = if id.anchors_right() {
            (screen.x + offset.x).max(SPACE_2)
        } else {
            offset.x
        };
        let wanted = Vec2::new(x.round(), offset.y.round());
        if current != wanted {
            manager.park(id, wanted);
        }
    }

    for (window, mut node, mut z) in &mut roots {
        let id = window.id;
        let state = *manager.slot(id);
        let display = if state.open {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        if !state.open {
            continue;
        }
        // A window is positioned from the left / top only. Foreign panels that
        // shipped anchored to the right have those sides released here so the
        // two anchors cannot fight.
        let left = Val::Px(state.pos.x);
        let top = Val::Px(state.pos.y);
        if node.position_type != PositionType::Absolute {
            node.position_type = PositionType::Absolute;
        }
        if node.left != left {
            node.left = left;
        }
        if node.top != top {
            node.top = top;
        }
        if node.right != Val::Auto {
            node.right = Val::Auto;
        }
        if node.bottom != Val::Auto {
            node.bottom = Val::Auto;
        }
        let wanted = ZIndex(WINDOW_Z_BASE + manager.depth(id));
        if *z != wanted {
            *z = wanted;
        }
    }
}

/// Drag a window by its title bar.
///
/// Movement is in whole texels (03 §9) and the bar is always kept on screen, so
/// a window can never be dragged somewhere it cannot be dragged back from.
pub fn drag_windows(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    ui_scale: Res<UiScale>,
    bars: Query<(&Interaction, &WindowTitleBar), With<Button>>,
    mut manager: ResMut<WindowManager>,
) {
    if !mouse.pressed(MouseButton::Left) {
        if manager.drag.is_some() {
            manager.drag = None;
        }
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    // Cursor is logical px; window positions are UI px (`kit::cursor_to_ui`).
    let cursor = cursor_to_ui(cursor, ui_scale.0);

    if manager.drag.is_none() {
        if !mouse.just_pressed(MouseButton::Left) {
            return;
        }
        let Some((_, bar)) = bars
            .iter()
            .find(|(i, _)| matches!(i, Interaction::Pressed | Interaction::Hovered))
        else {
            return;
        };
        let id = bar.id;
        let origin = manager.slot(id).pos;
        manager.raise(id);
        manager.drag = Some(WindowDrag {
            id,
            grab: cursor,
            origin,
        });
        return;
    }

    let Some(drag) = manager.drag else {
        return;
    };
    let screen = screen_to_ui(Vec2::new(window.width(), window.height()), ui_scale.0);
    let wanted = drag.origin + (cursor - drag.grab);
    // Keep at least the title bar reachable on every edge.
    let clamped = Vec2::new(
        wanted.x.clamp(-SPACE_2, (screen.x - SPACE_8_MIN).max(0.0)),
        wanted.y.clamp(0.0, (screen.y - TITLE_BAR_H).max(0.0)),
    );
    if clamped.distance_squared(manager.slot(drag.id).pos) > 0.25 {
        manager.place(drag.id, clamped);
    }
}

/// Smallest sliver of a window that must remain on screen horizontally.
const SPACE_8_MIN: f32 = 48.0;

/// Close box clicks.
pub fn window_close_clicks(
    interactions: Query<(&Interaction, &WindowCloseButton), (Changed<Interaction>, With<Button>)>,
    mut manager: ResMut<WindowManager>,
) {
    for (interaction, close) in &interactions {
        if *interaction == Interaction::Pressed {
            manager.close(close.id);
        }
    }
}

/// Clicking anywhere in a window raises it above the others.
pub fn raise_clicked_window(
    mouse: Res<ButtonInput<MouseButton>>,
    roots: Query<(&UiWindow, &Interaction)>,
    mut manager: ResMut<WindowManager>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    for (window, interaction) in &roots {
        if matches!(interaction, Interaction::Pressed | Interaction::Hovered) {
            manager.raise(window.id);
            return;
        }
    }
}

/// Paint the title bar and close box: the focused window reads brighter.
///
/// Two narrow queries with `Changed` filters on the hover half, so an idle
/// frame touches nothing.
pub fn update_window_chrome(
    manager: Res<WindowManager>,
    mut bars: Query<(&WindowTitleBar, &mut BackgroundColor)>,
    mut closes: Query<(&Interaction, &mut BorderColor), (Changed<Interaction>, With<WindowCloseButton>)>,
) {
    if manager.is_changed() {
        let top = manager.top();
        for (bar, mut bg) in &mut bars {
            let wanted = if top == Some(bar.id) { BG0 } else { BG1 };
            if bg.0 != wanted {
                *bg = BackgroundColor(wanted);
            }
        }
    }
    for (interaction, mut border) in &mut closes {
        let hovered = matches!(interaction, Interaction::Hovered | Interaction::Pressed);
        *border = control_border(false, hovered);
    }
}

/// `Esc` closes the top window and stops there.
///
/// Runs in `PreUpdate`, ahead of every other `Esc` consumer, and clears the key
/// press when it acts — one layer per press, exactly as 03 §10.1 requires.
pub fn close_top_window_on_escape(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut manager: ResMut<WindowManager>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    let Some(top) = manager.top() else {
        return;
    };
    manager.close(top);
    keys.clear_just_pressed(KeyCode::Escape);
}

/// Every panel that is up, in one comparable value.
///
/// The Settings overlay is a shell resource rather than a [`WindowId`], and it
/// is the only panel in the game the manager does not own — so it rides along
/// here rather than getting a second, near-identical system elsewhere.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PanelsOpen {
    windows: Vec<WindowId>,
    settings: bool,
}

impl PanelsOpen {
    fn opened_since(&self, before: &Self) -> bool {
        (self.settings && !before.settings)
            || self.windows.iter().any(|id| !before.windows.contains(id))
    }

    fn closed_since(&self, before: &Self) -> bool {
        (!self.settings && before.settings)
            || before.windows.iter().any(|id| !self.windows.contains(id))
    }
}

/// The one cue a frame's worth of change is worth — design 10 §5's sweep.
///
/// **At most one, and opening wins.** Opening B while A is already up is one
/// opening, not a close and two opens. A panel that *replaces* another closes
/// and opens in the same frame and is still one opening, because that is what
/// the player did. Anything else turns a click into a chord, and §1's first
/// rule is never to startle.
fn cue_for(before: &PanelsOpen, after: &PanelsOpen) -> Option<UiCue> {
    if after.opened_since(before) {
        Some(UiCue::PanelOpen)
    } else if after.closed_since(before) {
        Some(UiCue::PanelClose)
    } else {
        None
    }
}

/// What was on screen last frame, so [`panel_cues`] can spot the change.
#[derive(Resource, Debug, Default)]
pub struct PanelCueWatch {
    last: PanelsOpen,
    /// The first frame records rather than announces: a panel that was already
    /// up is not a panel the player just opened.
    seeded: bool,
}

/// Every panel's open and close sweep, from the one place visibility flips.
///
/// This diffs state rather than listening for events, so a cue can be a frame
/// late but can never be *missed* — including for a window opened by code that
/// had no idea sound existed, which is exactly the failure this replaces.
///
/// `SettingsPanel` is optional because `ui` runs headless in tests with no
/// shell.
pub fn panel_cues(
    manager: Res<WindowManager>,
    settings: Option<Res<crate::shell::SettingsPanel>>,
    mut watch: ResMut<PanelCueWatch>,
    mut cues: MessageWriter<UiCue>,
) {
    let now = PanelsOpen {
        windows: WindowId::ALL
            .iter()
            .copied()
            .filter(|id| manager.is_open(*id))
            .collect(),
        settings: settings.as_deref().is_some_and(|panel| panel.open),
    };
    if !watch.seeded {
        watch.last = now;
        watch.seeded = true;
        return;
    }
    if now == watch.last {
        return;
    }
    if let Some(cue) = cue_for(&watch.last, &now) {
        cues.write(cue);
    }
    watch.last = now;
}

/// Colours referenced by the chrome, kept live for the palette audit.
#[allow(dead_code)]
fn _palette_parity() -> [Color; 4] {
    [BG1, HI, OUTLINE, BALLAST_L]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manager with nothing up, for the stacking tests that predate the
    /// boot-open feed.
    fn empty() -> WindowManager {
        let mut m = WindowManager::new();
        m.close(WindowId::TownTalk);
        m
    }

    #[test]
    fn opening_raises_and_closing_pops_the_stack() {
        let mut m = empty();
        assert!(m.top().is_none());
        m.open(WindowId::Ledger);
        m.open(WindowId::TownTalk);
        assert_eq!(m.top(), Some(WindowId::TownTalk));
        m.raise(WindowId::Ledger);
        assert_eq!(m.top(), Some(WindowId::Ledger));
        m.close(WindowId::Ledger);
        assert_eq!(m.top(), Some(WindowId::TownTalk));
        m.close(WindowId::TownTalk);
        assert!(m.top().is_none());
    }

    #[test]
    fn town_talk_is_the_one_window_that_starts_open() {
        // DESIGN.md: the complaint feed is the diagnostic layer *and* the
        // emotional hook, and the opening nudge is spoken into it. Everything
        // else waits to be asked for.
        let m = WindowManager::new();
        assert!(m.is_open(WindowId::TownTalk));
        assert_eq!(m.top(), Some(WindowId::TownTalk), "and it is reachable by Esc");
        for id in WindowId::ALL.iter().filter(|id| **id != WindowId::TownTalk) {
            assert!(!m.is_open(*id), "{id:?} opens uninvited");
        }
    }

    #[test]
    fn closing_the_boot_feed_keeps_it_closed() {
        // Opened by the game, closed by the player, and it stays that way —
        // including the position they left it at.
        let mut m = WindowManager::new();
        m.place(WindowId::TownTalk, Vec2::new(64.0, 300.0));
        m.close(WindowId::TownTalk);
        assert!(!m.is_open(WindowId::TownTalk));
        assert!(m.top().is_none());
        m.open(WindowId::TownTalk);
        assert_eq!(m.slot(WindowId::TownTalk).pos, Vec2::new(64.0, 300.0));
    }

    #[test]
    fn a_window_remembers_where_it_was_left() {
        let mut m = empty();
        m.open(WindowId::Ledger);
        m.place(WindowId::Ledger, Vec2::new(310.0, 96.0));
        m.close(WindowId::Ledger);
        m.open(WindowId::Ledger);
        assert_eq!(m.slot(WindowId::Ledger).pos, Vec2::new(310.0, 96.0));
    }

    #[test]
    fn positions_are_always_whole_texels() {
        // 03 §9 — a panel on a half texel resamples and stops looking like art.
        let mut m = empty();
        m.place(WindowId::Alerts, Vec2::new(12.4, 33.6));
        let pos = m.slot(WindowId::Alerts).pos;
        assert_eq!(pos, Vec2::new(12.0, 34.0));
    }

    #[test]
    fn raising_a_closed_window_does_nothing() {
        let mut m = empty();
        m.raise(WindowId::Goals);
        assert!(m.top().is_none());
    }

    #[test]
    fn toggle_is_open_then_closed() {
        let mut m = empty();
        m.toggle(WindowId::Neighbours);
        assert!(m.is_open(WindowId::Neighbours));
        m.toggle(WindowId::Neighbours);
        assert!(!m.is_open(WindowId::Neighbours));
        assert!(m.top().is_none());
    }

    /// The open set for a manager, as [`panel_cues`] builds it.
    fn panels(manager: &WindowManager, settings: bool) -> PanelsOpen {
        PanelsOpen {
            windows: WindowId::ALL
                .iter()
                .copied()
                .filter(|id| manager.is_open(*id))
                .collect(),
            settings,
        }
    }

    #[test]
    fn opening_and_closing_a_panel_each_sweep_once() {
        let mut m = empty();
        let closed = panels(&m, false);
        m.open(WindowId::Ledger);
        let one_open = panels(&m, false);
        assert_eq!(cue_for(&closed, &one_open), Some(UiCue::PanelOpen));
        assert_eq!(cue_for(&one_open, &closed), Some(UiCue::PanelClose));
        assert_eq!(cue_for(&one_open, &one_open), None, "an idle frame is silent");
    }

    #[test]
    fn opening_a_second_panel_does_not_also_play_a_close() {
        // The brief's rule restated: B opening over A is one sweep, not
        // close-open-open. A chord where a click belongs is a startle.
        let mut m = empty();
        m.open(WindowId::Ledger);
        let before = panels(&m, false);
        m.open(WindowId::TownTalk);
        assert_eq!(cue_for(&before, &panels(&m, false)), Some(UiCue::PanelOpen));
    }

    #[test]
    fn a_panel_replacing_another_is_one_opening() {
        // Both flips land in the same frame; the player opened something, so
        // that is what they hear.
        let mut m = empty();
        m.open(WindowId::Ledger);
        let before = panels(&m, false);
        m.close(WindowId::Ledger);
        m.open(WindowId::Alerts);
        assert_eq!(cue_for(&before, &panels(&m, false)), Some(UiCue::PanelOpen));
    }

    #[test]
    fn raising_a_window_is_not_an_opening() {
        // Stacking order is not visibility. Clicking between two open windows
        // must be silent.
        let mut m = empty();
        m.open(WindowId::Ledger);
        m.open(WindowId::TownTalk);
        let before = panels(&m, false);
        m.raise(WindowId::Ledger);
        assert_eq!(cue_for(&before, &panels(&m, false)), None);
    }

    #[test]
    fn the_settings_overlay_sweeps_like_any_other_panel() {
        // It is the one panel the manager does not own, and it is still a panel.
        let m = empty();
        let shut = panels(&m, false);
        let up = panels(&m, true);
        assert_eq!(cue_for(&shut, &up), Some(UiCue::PanelOpen));
        assert_eq!(cue_for(&up, &shut), Some(UiCue::PanelClose));
    }

    #[test]
    fn closing_the_last_of_several_still_closes() {
        let mut m = empty();
        m.open(WindowId::Ledger);
        m.open(WindowId::Goals);
        let before = panels(&m, false);
        m.close(WindowId::Goals);
        assert_eq!(cue_for(&before, &panels(&m, false)), Some(UiCue::PanelClose));
    }

    /// Every cue written this frame, drained so the next frame starts clean.
    fn drain_cues(app: &mut App) -> Vec<UiCue> {
        app.world_mut()
            .resource_mut::<Messages<UiCue>>()
            .drain()
            .collect()
    }

    #[test]
    fn the_boot_open_feed_does_not_sweep() {
        // `WindowManager::new` puts Town Talk up before the player has clicked
        // anything, and 10 §1's first rule is never to startle. `panel_cues`
        // seeds its baseline on its first run rather than announcing it, which
        // is what makes the boot frame silent — asserted rather than assumed.
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .insert_resource(WindowManager::new())
            .init_resource::<PanelCueWatch>()
            .add_message::<UiCue>()
            .add_systems(Update, panel_cues);

        for _ in 0..3 {
            app.update();
            assert!(
                drain_cues(&mut app).is_empty(),
                "the boot-open feed swept on its own"
            );
        }

        // …and a panel the player really does open still sweeps.
        app.world_mut()
            .resource_mut::<WindowManager>()
            .open(WindowId::Ledger);
        app.update();
        assert_eq!(drain_cues(&mut app), vec![UiCue::PanelOpen]);
    }

    #[test]
    fn every_window_has_a_title_and_a_home() {
        for id in WindowId::ALL {
            assert!(!id.title().is_empty(), "{id:?} has no title");
            let offset = id.default_offset();
            assert!(offset.y >= TOP_CHROME_H, "{id:?} opens under the top chrome");
        }
    }
}
