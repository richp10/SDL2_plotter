//! What the user can ask the application to do. Menus, keys and gestures
//! are all translated into one of these; `Session::execute_action` is the
//! single place that acts on them.

use crate::Mode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppAction {
    SetMode(Mode),
    CycleMode,
    ToggleFollow,
    /// Freeze the chart file shown against automatic switching, or release.
    ToggleLock,
    /// Switch between the chosen chart layer and the last imagery layer.
    ToggleImagery,
    /// Open the chart manager window (the shell's, not the session's).
    OpenChartManager,
    FollowOn,
    FollowOff,
    ToggleGoto,
    ClearRoute,
    PreviousWaypoint,
    NextWaypoint,
    Quit,
    ClearTrack,
    ExportRouteGpx,
    ExportTrackGpx,
    ClearWaypoints,
    ExportWaypointsGpx,
    /// Entry `i` of `Catalogue::switchable_layers`, the layer switcher.
    ChooseLayer(usize),
    Pan(i32, i32),
    ZoomIn,
    ZoomOut,
    ShowWaypoints,
    HideWaypoints,
    ShowRoute,
    HideRoute,
    ShowTrack,
    HideTrack,
    ShowAis,
    HideAis,
    SetCourseUp(bool),
}

impl AppAction {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SetMode(_) => "SetMode",
            Self::CycleMode => "CycleMode",
            Self::ToggleFollow => "ToggleFollow",
            Self::ToggleLock => "ToggleLock",
            Self::ToggleImagery => "ToggleImagery",
            Self::OpenChartManager => "OpenChartManager",
            Self::FollowOn => "On",
            Self::FollowOff => "Off",
            Self::ToggleGoto => "ToggleGoto",
            Self::ClearRoute => "ClearRoute",
            Self::PreviousWaypoint => "PreviousWaypoint",
            Self::NextWaypoint => "NextWaypoint",
            Self::Quit => "Quit",
            Self::ClearTrack => "ClearTrack",
            Self::ExportRouteGpx => "ExportRouteGpx",
            Self::ExportTrackGpx => "ExportTrackGpx",
            Self::ClearWaypoints => "ClearWaypoints",
            Self::ExportWaypointsGpx => "ExportWaypointsGpx",
            Self::ChooseLayer(_) => "ChooseLayer",
            Self::Pan(_, _) => "Pan",
            Self::ZoomIn => "ZoomIn",
            Self::ZoomOut => "ZoomOut",
            Self::ShowWaypoints => "ShowWaypoints",
            Self::HideWaypoints => "HideWaypoints",
            Self::ShowRoute => "ShowRoute",
            Self::HideRoute => "HideRoute",
            Self::ShowTrack => "ShowTrack",
            Self::HideTrack => "HideTrack",
            Self::ShowAis => "ShowAis",
            Self::HideAis => "HideAis",
            Self::SetCourseUp(_) => "SetCourseUp",
        }
    }
}
