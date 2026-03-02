use godot::builtin::{VarArray, VarDictionary, Variant, VariantType};
use godot::classes::notify::ControlNotification;
use godot::classes::{
    Button, Control, Engine, IControl, Label, Node, OptionButton, Os, PackedScene, PanelContainer,
    ScrollContainer, VBoxContainer,
};
use godot::global::godot_warn;
use godot::prelude::*;
use std::collections::HashMap;

const UI_SCENE_PATH: &str = "res://addons/godot_cef/inspector/cef_ipc_inspector_ui.tscn";
const ITEM_SCENE_PATH: &str = "res://addons/godot_cef/inspector/ipc_message_item.tscn";
const MAX_MESSAGES: usize = 500;
const DEFAULT_EXPANDED_MAX_CHARS: usize = 120;
const DEFAULT_EXPANDED_MAX_LINES: usize = 3;
const PREVIEW_MAX_CHARS: usize = 240;
const PREVIEW_MAX_LINES: usize = 3;

#[derive(Clone, Debug)]
struct InspectorMessage {
    id: i64,
    direction: String,
    lane: String,
    body: String,
    timestamp_unix_ms: i64,
    body_size_bytes: i64,
}

#[derive(GodotClass)]
#[class(base=Control, tool)]
pub struct CefIpcInspector {
    base: Base<Control>,

    #[export]
    #[var(
        get = get_target_cef_texture,
        set = set_target_cef_texture
    )]
    target_cef_texture: Option<Gd<crate::cef_texture::CefTexture>>,

    ui_root: Option<Gd<Control>>,
    item_scene: Option<Gd<PackedScene>>,
    toggle_button: Option<Gd<Button>>,
    panel: Option<Gd<PanelContainer>>,
    title_label: Option<Gd<Label>>,
    direction_filter: Option<Gd<OptionButton>>,
    clear_button: Option<Gd<Button>>,
    scroll: Option<Gd<ScrollContainer>>,
    message_list: Option<Gd<VBoxContainer>>,
    empty_label: Option<Gd<Label>>,

    is_open: bool,
    selected_filter: i32,
    messages: Vec<InspectorMessage>,
    expanded_by_id: HashMap<i64, bool>,
    next_message_id: i64,
    cef_texture: Option<Gd<Node>>,
}

#[godot_api]
impl IControl for CefIpcInspector {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            target_cef_texture: None,
            ui_root: None,
            item_scene: None,
            toggle_button: None,
            panel: None,
            title_label: None,
            direction_filter: None,
            clear_button: None,
            scroll: None,
            message_list: None,
            empty_label: None,
            is_open: false,
            selected_filter: 0,
            messages: Vec::new(),
            expanded_by_id: HashMap::new(),
            next_message_id: 1,
            cef_texture: None,
        }
    }

    fn on_notification(&mut self, what: ControlNotification) {
        if what == ControlNotification::READY {
            self.on_ready();
        }
    }
}

#[godot_api]
impl CefIpcInspector {
    #[func]
    fn on_ready(&mut self) {
        if !Self::is_enabled_for_current_runtime() {
            return;
        }

        self.ensure_ui();
        self.bind_ui_signals();
        self.configure_filter();
        self.update_ui_state();
        self.try_bind_cef_texture();
    }

    #[func]
    fn _on_toggle_pressed(&mut self) {
        self.is_open = !self.is_open;
        self.update_ui_state();
        if self.is_open {
            self.render_messages();
        }
    }

    #[func]
    fn _on_clear_pressed(&mut self) {
        self.messages.clear();
        self.expanded_by_id.clear();
        self.next_message_id = 1;
        self.render_messages();
        self.update_title();
    }

    #[func]
    fn _on_filter_changed(&mut self, idx: i32) {
        self.selected_filter = idx;
        self.render_messages();
    }

    #[func]
    fn _on_child_entered_tree(&mut self, _node: Gd<Node>) {
        if self.cef_texture.is_none() {
            self.try_bind_cef_texture();
        }
    }

    #[func]
    fn _on_debug_ipc_message(&mut self, event: Variant) {
        if event.get_type() != VariantType::DICTIONARY {
            return;
        }

        let raw = event.to::<VarDictionary>();
        let entry = InspectorMessage {
            id: self.next_message_id,
            direction: Self::dict_get_string(&raw, "direction"),
            lane: Self::dict_get_string(&raw, "lane"),
            body: Self::dict_get_string(&raw, "body"),
            timestamp_unix_ms: Self::dict_get_i64(&raw, "timestamp_unix_ms"),
            body_size_bytes: Self::dict_get_i64(&raw, "body_size_bytes"),
        };
        self.next_message_id += 1;

        self.messages.push(entry);
        if self.messages.len() > MAX_MESSAGES {
            if let Some(removed) = self.messages.first() {
                self.expanded_by_id.remove(&removed.id);
            }
            self.messages.remove(0);
        }

        self.update_title();
        if self.is_open {
            self.render_messages();
        }
    }

    #[func]
    fn _on_item_toggle_pressed(&mut self, id: i64) {
        let expanded = self.expanded_by_id.get(&id).copied().unwrap_or(false);
        self.expanded_by_id.insert(id, !expanded);
        self.render_messages();
    }

    #[func]
    fn get_target_cef_texture(&self) -> Option<Gd<crate::cef_texture::CefTexture>> {
        self.target_cef_texture.clone()
    }

    #[func]
    fn set_target_cef_texture(&mut self, target: Option<Gd<crate::cef_texture::CefTexture>>) {
        self.target_cef_texture = target;
    }

    fn is_enabled_for_current_runtime() -> bool {
        Os::singleton().is_debug_build() || Engine::singleton().is_editor_hint()
    }

    fn ensure_ui(&mut self) {
        if self.ui_root.is_some() {
            return;
        }

        let packed = match try_load::<PackedScene>(UI_SCENE_PATH) {
            Ok(scene) => scene,
            Err(err) => {
                godot_warn!(
                    "[CefIpcInspector] Failed to load UI scene {}: {}",
                    UI_SCENE_PATH,
                    err
                );
                return;
            }
        };

        let Some(mut ui_root) = packed.try_instantiate_as::<Control>() else {
            godot_warn!("[CefIpcInspector] UI root is not a Control");
            return;
        };

        ui_root.set_name("__ipc_inspector_ui");
        self.base_mut().add_child(&ui_root);

        self.toggle_button = ui_root.try_get_node_as("ToggleButton");
        self.panel = ui_root.try_get_node_as("InspectorPanel");
        self.title_label = ui_root.try_get_node_as("InspectorPanel/Margin/VBox/Header/TitleLabel");
        self.direction_filter =
            ui_root.try_get_node_as("InspectorPanel/Margin/VBox/Header/DirectionFilter");
        self.clear_button =
            ui_root.try_get_node_as("InspectorPanel/Margin/VBox/Header/ClearButton");
        self.scroll = ui_root.try_get_node_as("InspectorPanel/Margin/VBox/Scroll");
        self.message_list =
            ui_root.try_get_node_as("InspectorPanel/Margin/VBox/Scroll/MessageList");
        self.empty_label = ui_root.try_get_node_as("InspectorPanel/Margin/VBox/EmptyLabel");

        self.ui_root = Some(ui_root);

        self.item_scene = match try_load::<PackedScene>(ITEM_SCENE_PATH) {
            Ok(scene) => Some(scene),
            Err(err) => {
                godot_warn!(
                    "[CefIpcInspector] Failed to load item scene {}: {}",
                    ITEM_SCENE_PATH,
                    err
                );
                None
            }
        };
    }

    fn bind_ui_signals(&mut self) {
        if let Some(mut toggle) = self.toggle_button.clone() {
            let callable = self.base().callable("_on_toggle_pressed");
            if !toggle.is_connected("pressed", &callable) {
                toggle.connect("pressed", &callable);
            }
        }

        if let Some(mut clear) = self.clear_button.clone() {
            let callable = self.base().callable("_on_clear_pressed");
            if !clear.is_connected("pressed", &callable) {
                clear.connect("pressed", &callable);
            }
        }

        if let Some(mut filter) = self.direction_filter.clone() {
            let callable = self.base().callable("_on_filter_changed");
            if !filter.is_connected("item_selected", &callable) {
                filter.connect("item_selected", &callable);
            }
        }

        let callable = self.base().callable("_on_child_entered_tree");
        let mut base = self.base_mut();
        if !base.is_connected("child_entered_tree", &callable) {
            base.connect("child_entered_tree", &callable);
        }
    }

    fn configure_filter(&mut self) {
        if let Some(mut filter) = self.direction_filter.clone() {
            filter.clear();
            filter.add_item("All");
            filter.add_item("Incoming");
            filter.add_item("Outgoing");
            filter.select(0);
            self.selected_filter = 0;
        }
    }

    fn update_ui_state(&mut self) {
        if let Some(mut panel) = self.panel.clone() {
            panel.set_visible(self.is_open);
        }

        if let Some(mut toggle) = self.toggle_button.clone() {
            toggle.set_text(if self.is_open { "Close" } else { "IPC" });
        }

        self.update_title();
    }

    fn update_title(&mut self) {
        if let Some(mut title) = self.title_label.clone() {
            let text = format!("IPC Inspector ({})", self.messages.len());
            title.set_text(&text);
        }
    }

    fn try_bind_cef_texture(&mut self) {
        if self.cef_texture.is_some() {
            return;
        }

        let target_cef = match self.target_cef_texture.clone() {
            Some(target) => target,
            None => {
                if let Some(mut empty) = self.empty_label.clone()
                    && self.messages.is_empty()
                {
                    empty.set_text("Assign target_cef_texture to a CefTexture node.");
                }
                return;
            }
        };

        let mut target = target_cef.upcast::<Node>();
        let debug_callable = self.base().callable("_on_debug_ipc_message");
        if !target.is_connected("debug_ipc_message", &debug_callable) {
            target.connect("debug_ipc_message", &debug_callable);
        }
        self.cef_texture = Some(target);

        if let Some(mut empty) = self.empty_label.clone()
            && self.messages.is_empty()
        {
            empty.set_text("Waiting for IPC messages...");
        }
    }

    fn render_messages(&mut self) {
        let Some(mut list) = self.message_list.clone() else {
            return;
        };

        let children = list.get_children();
        for mut child in children.iter_shared() {
            child.queue_free();
        }

        let mut visible_count = 0usize;
        let messages: Vec<InspectorMessage> = self.messages.clone();
        for msg in &messages {
            if !self.passes_filter(msg) {
                continue;
            }
            visible_count += 1;
            let card = self.build_message_card(msg);
            list.add_child(&card);
        }

        if let Some(mut empty) = self.empty_label.clone() {
            let is_empty = visible_count == 0;
            empty.set_visible(is_empty);
            if is_empty && self.cef_texture.is_some() {
                empty.set_text("No messages match the current filter.");
            }
        }
    }

    fn passes_filter(&self, msg: &InspectorMessage) -> bool {
        match self.selected_filter {
            1 => msg.direction == "to_renderer",
            2 => msg.direction == "to_godot",
            _ => true,
        }
    }

    fn build_message_card(&mut self, msg: &InspectorMessage) -> Gd<PanelContainer> {
        let default_expanded = Self::default_expanded(&msg.body);
        let expanded = self
            .expanded_by_id
            .get(&msg.id)
            .copied()
            .unwrap_or(default_expanded);
        self.expanded_by_id.insert(msg.id, expanded);

        let card = self
            .item_scene
            .clone()
            .and_then(|scene| scene.try_instantiate_as::<PanelContainer>())
            .unwrap_or_else(PanelContainer::new_alloc);

        let header_text = self.format_header(msg);
        if let Some(mut header) = card.try_get_node_as::<Label>("Margin/VBox/HeaderLabel") {
            header.set_text(&header_text);
        }

        let body_text = if expanded {
            msg.body.clone()
        } else {
            Self::preview_text(&msg.body)
        };
        if let Some(mut body) = card.try_get_node_as::<Label>("Margin/VBox/BodyLabel") {
            body.set_text(&body_text);
        }

        if let Some(mut toggle) = card.try_get_node_as::<Button>("Margin/VBox/ToggleButton") {
            if default_expanded {
                toggle.set_visible(false);
            } else {
                toggle.set_visible(true);
                toggle.set_text(if expanded { "Show less" } else { "Show more" });
                let callable = self.base().callable("_on_item_toggle_pressed");
                let mut args = VarArray::new();
                args.push(&msg.id.to_variant());
                let bound = callable.bindv(&args);
                toggle.connect("pressed", &bound);
            }
        }

        card
    }

    fn format_header(&self, msg: &InspectorMessage) -> String {
        format!(
            "{}  |  {}  |  {}  |  {} B",
            Self::format_timestamp_ms(msg.timestamp_unix_ms),
            Self::direction_label(&msg.direction),
            msg.lane.to_uppercase(),
            msg.body_size_bytes
        )
    }

    fn dict_get_string(dict: &VarDictionary, key: &str) -> String {
        dict.get(key)
            .map(|v| v.stringify().to_string())
            .unwrap_or_default()
    }

    fn dict_get_i64(dict: &VarDictionary, key: &str) -> i64 {
        dict.get(key)
            .and_then(|v| v.stringify().to_string().parse::<i64>().ok())
            .unwrap_or(0)
    }

    fn default_expanded(body: &str) -> bool {
        let line_count = body.matches('\n').count() + 1;
        body.chars().count() <= DEFAULT_EXPANDED_MAX_CHARS
            && line_count <= DEFAULT_EXPANDED_MAX_LINES
    }

    fn preview_text(body: &str) -> String {
        let mut out_lines = Vec::new();
        for line in body.lines().take(PREVIEW_MAX_LINES) {
            out_lines.push(line);
        }

        let mut preview = out_lines.join("\n");
        if preview.chars().count() > PREVIEW_MAX_CHARS {
            preview = preview.chars().take(PREVIEW_MAX_CHARS).collect();
        }

        if preview != body {
            preview.push_str("...");
        }

        preview
    }

    fn format_timestamp_ms(ms: i64) -> String {
        if ms <= 0 {
            return "--:--:--.---".to_string();
        }

        let sec = ms / 1000;
        let milli = ms % 1000;
        let h = (sec / 3600) % 24;
        let m = (sec / 60) % 60;
        let s = sec % 60;
        format!("{:02}:{:02}:{:02}.{:03}", h, m, s, milli)
    }

    fn direction_label(direction: &str) -> &'static str {
        match direction {
            "to_godot" => "Outgoing",
            "to_renderer" => "Incoming",
            _ => "Unknown",
        }
    }
}
