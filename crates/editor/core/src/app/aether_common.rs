//! Shared Aether view projection helpers.
//!
//! These are the data-only row/style containers used by adopted Aether views.
//! View-specific behavior stays in each view/model module.

use std::collections::BTreeMap;
use std::fmt;

use az_editor_inspector::ReflectedEditBinding;
use az_proto_project::vnext::{PrefabEditCommand, ReflectedValueEnvelope};

pub(crate) fn non_empty_string_or(preferred: impl AsRef<str>, fallback: &str) -> String {
    let preferred = preferred.as_ref().trim();
    if preferred.is_empty() {
        fallback.to_owned()
    } else {
        preferred.to_owned()
    }
}

pub(crate) fn replace_string_if_changed(slot: &mut String, value: &str) -> bool {
    if slot == value {
        return false;
    }
    slot.clear();
    slot.push_str(value);
    true
}

pub(crate) fn asset_display_name(source_path: &str) -> String {
    source_path
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(source_path)
        .to_owned()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AetherItems(pub(crate) Vec<AetherItem>);

/// Typed editor command carried by an Aether menu projection.
///
/// `AetherItem::key` remains presentation identity for the adopted view, but
/// executable menu rows carry this enum so activation never reparses display
/// strings into behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AetherMenuAction {
    NewProject,
    OpenProject,
    Save,
    RefreshAssets,
    RefreshGameDataCatalog,
    BackToProjectLauncher,
    Quit,
    Undo,
    Redo,
    Preferences,
    ToggleOutliner,
    ToggleInspector,
    ToggleAssetBrowser,
    ToggleConsole,
    ToggleSessionPanel,
    ToggleGraphPanel,
    RefreshAuthoredOutline,
    ResetLayout,
    ToggleFullscreen,
    LaunchEditorWorld,
    StopEditorWorld,
    RefreshRuntimeStatus,
    RefreshViewportFrame,
    RefreshRuntimeProjections,
    RefreshSessionStatus,
    StartSessionServices,
    StopSessionServices,
    RecoverSession,
    ForceRecoverSession,
    About,
}

impl AetherItems {
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, AetherItem> {
        self.0.iter()
    }
}

impl fmt::Display for AetherItems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let labels: Vec<&str> = self
            .0
            .iter()
            .filter_map(|item| {
                if item.label.is_empty() {
                    None
                } else {
                    Some(item.label.as_str())
                }
            })
            .collect();
        if labels.is_empty() {
            write!(f, "{}", self.0.len())
        } else {
            write!(f, "{}", labels.join(", "))
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AetherItem {
    pub(crate) menu_action: Option<AetherMenuAction>,
    pub(crate) any_label: String,
    pub(crate) badge: String,
    pub(crate) blend_val: String,
    pub(crate) blurb: String,
    pub(crate) body: String,
    pub(crate) caret: String,
    pub(crate) cat: String,
    pub(crate) check_color: String,
    pub(crate) check_icon: String,
    pub(crate) color: String,
    pub(crate) comp: String,
    pub(crate) count: String,
    pub(crate) display: String,
    pub(crate) document_id: String,
    pub(crate) ext: String,
    pub(crate) field: String,
    pub(crate) file: String,
    pub(crate) frames: String,
    pub(crate) from: String,
    pub(crate) from_color: String,
    pub(crate) from_icon: String,
    pub(crate) g: String,
    pub(crate) head: String,
    pub(crate) head_icon: String,
    pub(crate) hex: String,
    pub(crate) ico: String,
    pub(crate) icon: String,
    pub(crate) icon_color: String,
    pub(crate) id: String,
    pub(crate) idx: String,
    pub(crate) idx_label: String,
    pub(crate) key: String,
    pub(crate) key_name: String,
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) leader: String,
    pub(crate) lock_icon: String,
    pub(crate) loop_icon: String,
    pub(crate) meta: String,
    pub(crate) method: String,
    pub(crate) r#mod: String,
    pub(crate) motion: String,
    pub(crate) motion_label: String,
    pub(crate) ms: String,
    pub(crate) msg: String,
    pub(crate) mute_icon: String,
    pub(crate) n: String,
    pub(crate) name: String,
    pub(crate) name_color: String,
    pub(crate) object_id: String,
    pub(crate) op: String,
    pub(crate) out: String,
    pub(crate) priority: String,
    pub(crate) proj_placeholder: String,
    pub(crate) proj_type: String,
    pub(crate) rv_kind: String,
    pub(crate) rv_kind_color: String,
    pub(crate) rv_kind_icon: String,
    pub(crate) rv_name: String,
    pub(crate) rv_owner: String,
    pub(crate) rv_row_type: String,
    pub(crate) shortcut: String,
    pub(crate) size: String,
    pub(crate) src: String,
    pub(crate) src_count: String,
    pub(crate) stage: String,
    pub(crate) status: String,
    pub(crate) sub: String,
    pub(crate) sub_label: String,
    pub(crate) table: String,
    pub(crate) tag: String,
    pub(crate) text: String,
    pub(crate) time: String,
    pub(crate) title: String,
    pub(crate) to: String,
    pub(crate) toggle_title: String,
    pub(crate) type_color: String,
    pub(crate) type_icon: String,
    pub(crate) type_label: String,
    pub(crate) schema_type: String,
    pub(crate) unit: String,
    pub(crate) used: String,
    pub(crate) val: String,
    pub(crate) val_val: String,
    pub(crate) value: String,
    pub(crate) ver: String,
    pub(crate) vis_color: String,
    pub(crate) vis_icon: String,
    pub(crate) x: String,
    pub(crate) x_val: String,
    pub(crate) y: String,
    pub(crate) y_val: String,
    pub(crate) z: String,
    pub(crate) depth: u32,
    pub(crate) disabled: bool,
    pub(crate) active: bool,
    pub(crate) add_show: bool,
    pub(crate) anim_blend_show: bool,
    pub(crate) anim_event_selected: bool,
    pub(crate) anim_in_jump: bool,
    pub(crate) anim_insp_motion: bool,
    pub(crate) anim_insp_sample: bool,
    pub(crate) anim_insp_trans: bool,
    pub(crate) anim_mirror_badge: bool,
    pub(crate) anim_tab_graph: bool,
    pub(crate) anim_tab_motions: bool,
    pub(crate) anim_tab_sets: bool,
    pub(crate) anim_tab_skeleton: bool,
    pub(crate) anim_trans: bool,
    pub(crate) any_show: bool,
    pub(crate) asset_grid: bool,
    pub(crate) asset_list: bool,
    pub(crate) can_add: bool,
    pub(crate) chips_show: bool,
    pub(crate) clip: bool,
    pub(crate) desc: bool,
    pub(crate) dot_show: bool,
    pub(crate) ent_is_prefab: bool,
    pub(crate) gd_cat_menu: bool,
    pub(crate) gd_field_shared_multi: bool,
    pub(crate) gd_has_incoming: bool,
    pub(crate) gd_is_ref: bool,
    pub(crate) gd_managers_view: bool,
    pub(crate) gd_no_incoming: bool,
    pub(crate) gd_ref_menu: bool,
    pub(crate) gd_schemas_view: bool,
    pub(crate) gd_sel_manager: bool,
    pub(crate) gd_sel_schema: bool,
    pub(crate) gd_sel_table: bool,
    pub(crate) gd_show_field_tab: bool,
    pub(crate) gd_show_grid: bool,
    pub(crate) gd_show_manager: bool,
    pub(crate) gd_show_schema: bool,
    pub(crate) gd_tab_field: bool,
    pub(crate) gd_tab_manager: bool,
    pub(crate) gd_tab_schema: bool,
    pub(crate) gd_tab_table: bool,
    pub(crate) gd_tables_view: bool,
    pub(crate) gd_type_menu: bool,
    pub(crate) has_badge: bool,
    pub(crate) has_caret: bool,
    pub(crate) has_children: bool,
    pub(crate) has_sublevels: bool,
    pub(crate) has_tag: bool,
    pub(crate) has_val: bool,
    pub(crate) ik_show: bool,
    pub(crate) is_about: bool,
    pub(crate) is_asset: bool,
    pub(crate) is_bool: bool,
    pub(crate) is_chips: bool,
    pub(crate) is_color: bool,
    pub(crate) is_entry: bool,
    pub(crate) is_enum: bool,
    pub(crate) is_event: bool,
    pub(crate) is_exit: bool,
    pub(crate) is_head: bool,
    pub(crate) is_key: bool,
    pub(crate) is_keybind: bool,
    pub(crate) is_num: bool,
    pub(crate) is_param: bool,
    pub(crate) is_playing: bool,
    pub(crate) is_ref: bool,
    pub(crate) is_row: bool,
    pub(crate) is_seg: bool,
    pub(crate) is_select: bool,
    pub(crate) is_settings: bool,
    pub(crate) is_slider: bool,
    pub(crate) is_state: bool,
    pub(crate) is_str: bool,
    pub(crate) is_sub: bool,
    pub(crate) is_tex: bool,
    pub(crate) is_text: bool,
    pub(crate) is_toggle: bool,
    pub(crate) is_track: bool,
    pub(crate) is_trigger: bool,
    pub(crate) is_vec3: bool,
    pub(crate) label_show: bool,
    pub(crate) layout_open: bool,
    pub(crate) left_hierarchy: bool,
    pub(crate) left_layers: bool,
    pub(crate) level_dirty: bool,
    pub(crate) level_open: bool,
    pub(crate) lock_show: bool,
    pub(crate) major: bool,
    pub(crate) menu_open: bool,
    pub(crate) mgr_ins_composite: bool,
    pub(crate) mgr_ins_editable: bool,
    pub(crate) mgr_ins_has_deps: bool,
    pub(crate) mgr_ins_has_used_by: bool,
    pub(crate) mgr_ins_has_validation: bool,
    pub(crate) mgr_ins_is_auto: bool,
    pub(crate) mgr_is_authored: bool,
    pub(crate) mgr_is_auto: bool,
    pub(crate) mgr_key_menu_open: bool,
    pub(crate) mgr_owner_menu_open: bool,
    pub(crate) modal_open: bool,
    pub(crate) mode_animation: bool,
    pub(crate) mode_data: bool,
    pub(crate) mode_materials: bool,
    pub(crate) mode_scene: bool,
    pub(crate) mode_scripting: bool,
    pub(crate) mode_sequencer: bool,
    pub(crate) motion_show: bool,
    pub(crate) multi: bool,
    pub(crate) not_last: bool,
    pub(crate) notsep: bool,
    pub(crate) on: bool,
    pub(crate) open: bool,
    pub(crate) pipe_active: bool,
    pub(crate) pipe_open: bool,
    pub(crate) pk_icon: bool,
    pub(crate) prefab_overridden: bool,
    pub(crate) req: bool,
    pub(crate) right_details: bool,
    pub(crate) right_prefab: bool,
    pub(crate) running: bool,
    pub(crate) s1: bool,
    pub(crate) s2: bool,
    pub(crate) s3: bool,
    pub(crate) s4: bool,
    pub(crate) select_popover_open: bool,
    pub(crate) selected: bool,
    pub(crate) sep: bool,
    pub(crate) show_combine: bool,
    pub(crate) show_stats: bool,
    pub(crate) single: bool,
    pub(crate) sub_show: bool,
    pub(crate) tab_assets: bool,
    pub(crate) tab_console: bool,
    pub(crate) tab_gems: bool,
    pub(crate) tab_output: bool,
    pub(crate) tab_profiler: bool,
    pub(crate) uni: bool,
    pub(crate) via: bool,
    pub(crate) view_menu_open: bool,
    pub(crate) weight_show: bool,
    pub(crate) wiz: bool,
    pub(crate) wiz_open: bool,
    pub(crate) cells: AetherItems,
    pub(crate) chips: AetherItems,
    pub(crate) combine_opts: AetherItems,
    pub(crate) cond_list: AetherItems,
    pub(crate) dup_opts: AetherItems,
    pub(crate) ev_opts: AetherItems,
    pub(crate) events: AetherItems,
    pub(crate) fields: AetherItems,
    pub(crate) group_opts: AetherItems,
    pub(crate) ik_chips: AetherItems,
    pub(crate) interp_opts: AetherItems,
    pub(crate) interrupt_opts: AetherItems,
    pub(crate) items: AetherItems,
    pub(crate) key_field_opts: AetherItems,
    pub(crate) keys: AetherItems,
    pub(crate) kind_cards: AetherItems,
    pub(crate) op_opts: AetherItems,
    pub(crate) opts: AetherItems,
    pub(crate) owner_opts: AetherItems,
    pub(crate) param_opts: AetherItems,
    pub(crate) props: AetherItems,
    pub(crate) rows: AetherItems,
    pub(crate) rv_code_lines: AetherItems,
    pub(crate) rv_sources: AetherItems,
    pub(crate) sections: AetherItems,
    pub(crate) steps: AetherItems,
    pub(crate) summary: AetherItems,
    pub(crate) tokens: AetherItems,
    pub(crate) transform_chips: AetherItems,
    pub(crate) type_opts: AetherItems,
    pub(crate) wiz_assets: AetherItems,
    pub(crate) wiz_families: AetherItems,
    pub(crate) wiz_managers: AetherItems,
    pub(crate) wiz_tables: AetherItems,
    pub(crate) badge_style: AetherStyle,
    pub(crate) btn_style: AetherStyle,
    pub(crate) card_style: AetherStyle,
    pub(crate) caret_style: AetherStyle,
    pub(crate) dot_style: AetherStyle,
    pub(crate) icon_style: AetherStyle,
    pub(crate) name_style: AetherStyle,
    pub(crate) row_style: AetherStyle,
    pub(crate) style: AetherStyle,
    pub(crate) tag_style: AetherStyle,
    pub(crate) thumb_style: AetherStyle,
    pub(crate) vis_style: AetherStyle,
    pub(crate) style_fields: BTreeMap<String, AetherStyle>,
    pub(crate) edit_binding: Option<ReflectedEditBinding>,
    pub(crate) edit_command: Option<PrefabEditCommand>,
    pub(crate) edit_type_path: String,
    pub(crate) edit_text_quoted: bool,
    pub(crate) edit_value: Option<ReflectedValueEnvelope>,
    pub(crate) x_binding: Option<ReflectedEditBinding>,
    pub(crate) y_binding: Option<ReflectedEditBinding>,
    pub(crate) z_binding: Option<ReflectedEditBinding>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AetherStyle(BTreeMap<String, String>);

impl AetherStyle {
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn from_pairs(pairs: &[(&str, String)]) -> Self {
        Self(
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
        )
    }
}

impl AetherItem {
    pub(crate) fn style_named(&self, field: &str) -> AetherStyle {
        self.style_fields.get(field).cloned().unwrap_or_default()
    }
}
