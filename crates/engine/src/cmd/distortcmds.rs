//! Commands behind the distortion tools: width points, Liquify, Puppet Warp and the Perspective
//! Grid. The geometry kernels live in `vectorcraft_tools::distort` (shared with the tools).
//!
//! The perspective grid definition is document data stored under
//! `Document.unknown["perspectiveGrid"]` (read/written by [`grid_of`] / [`store_grid`]).

mod width;
use width::{width_point_copy, width_point_remove, width_point_set, width_profile_set};
mod liquify;
pub(crate) use liquify::LiquifyStroke;
use liquify::{liquify, points_of};
mod puppet;
pub(crate) use puppet::drop_puppet_pins;
use puppet::{puppet_pins, puppet_warp};
mod perspective;
pub use perspective::perspective_click;
use perspective::{
    attach, edit_text, grid_set, grid_show, persp_draw, persp_move, persp_nudge, persp_transform, plane_match, plane_move, plane_set, release,
};
pub(crate) use perspective::{finish_edit_text, grid_of, perspective_rewrite, silent, store_grid, transform_again};

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, PuppetPin, WidthProfile};
use vectorcraft_geom::{Affine, Homography, PathData, Point, Rect};
use vectorcraft_tools::distort::liquify::{Dabber, LiquifyParams, PathStroke, Sample, reach_bounds};
use vectorcraft_tools::distort::perspective::{self as persp, PerspectiveGrid, Plane};
use vectorcraft_tools::distort::{PinSet, arap, collect_points, mesh_for, project_node, stroke_owner, warp_from_rest, warp_node_with};

use super::edit::{duplicate_in, selected_roots};
use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "stroke.widthPoint.set",
            "Width Point",
            [],
            None,
            "{id, t: 0..1 (fraction of the path length), left, right: side widths in pt, index?: width point to edit or move, adjustAdjoining?: bool (with index: the nearest points either side change their widths in proportion)} → {index}. Without index a point already at t is replaced; a point moved onto another one's t joins it as a discontinuous point (on the side it came from), so the width steps there. Creates a uniform profile first if needed",
            has_doc,
            width_point_set
        ),
        cmd!(
            "stroke.widthPoint.remove",
            "Delete Width Point",
            [],
            None,
            "{id, index | indices: [..]} remove width points (removing the last one leaves a uniform stroke)",
            has_doc,
            width_point_remove
        ),
        cmd!(
            "stroke.widthPoint.copy",
            "Copy Width Point",
            [],
            None,
            "{id, index: the width point to copy, t: 0..1 where the copy goes} → {index} (Alt-drag with the Width tool). Landing on another point makes a discontinuous point, as a stroke.widthPoint.set move does",
            has_doc,
            width_point_copy
        ),
        cmd!(
            "stroke.widthProfile.set",
            "Width Profile",
            [],
            None,
            "{ids?|id?, points: [[t, left, right]…] (width factors, 1 = the stroke weight) | null = uniform}",
            has_doc,
            width_profile_set
        ),
        cmd!(
            "object.liquify",
            "Liquify",
            [],
            None,
            "{tool: warp|twirl|pucker|bloat|scallop|crystallize|wrinkle, points: [[x,y,pressure?]…] (the brush stroke; pen pressure 0..1, default 1), diameter?|width?, height? (pt, default 100), angle?, intensity? (0..1 or %), usePressure? (each point's pressure is the intensity there), detail? (1..10), simplify? (0..100) and simplifyOn? (default true; warp, twirl, pucker, bloat), rate? (twirl °), complexity? (0..15), horizontal?, vertical? (wrinkle, 0..1 or %), affectAnchors?, affectIn?, affectOut? (scallop, crystallize, wrinkle; default true), ids? (default: selection, else every path the brush touches)}",
            has_doc,
            liquify
        ),
        cmd!(
            "object.puppetWarp",
            "Puppet Warp",
            [],
            None,
            "{id?|ids? (default: selection), pins: [[x,y]…] (current pin positions), moved: [[x,y]…] (targets, same length), angles?: [deg|null…] (same length: the turn the art takes around each pin, as Alt-dragging around a pin does; null leaves it free), expand?: pt, rest?: bool} as-rigid-as-possible mesh warp of anchors and handles. rest: true = the Puppet Warp tool's pins: `pins` are points on the shape the pins started from (object.puppetWarp.pins), each must be on its mesh, the warp replaces the previous one instead of adding to it, and the pins are kept with the document for Undo (an empty list removes them all, leaving the art as it is)",
            has_doc,
            puppet_warp
        ),
        cmd!(
            "object.puppetWarp.pins",
            "Puppet Warp Pins",
            [],
            None,
            "{id?|ids? (default: selection), expand?: pt} → {ids, pins, moved, angles, expand, rest: true, auto} the Puppet Warp pins on the art: pins = where each sits on the shape they started from, moved = where it is now, angles = the turn it holds (deg, null = free); auto: none placed yet (the tool's automatic pins: the centre and the end of each limb). Change moved/angles (or add/remove pins) and pass it back to object.puppetWarp",
            has_doc,
            puppet_pins
        ),
        cmd!(
            "perspective.grid.set",
            "Define Perspective Grid",
            [],
            None,
            "{kind?: 1|2|3, origin?: [x,y], horizon?, vpLeft?, vpRight?, vpVertical?: [x,y], distance?, cell?, extent?, height?, visible?, plane?, leftOffset?, rightOffset?, groundOffset?: pt (planes moved along their normals), reproject?: bool} merge into the document's grid. Objects in perspective stay where they are, as in the reference app; with reproject they move with the grid (each keeps its place on its plane)",
            has_doc,
            grid_set
        ),
        cmd!(
            "perspective.grid.preset",
            "Perspective Grid Preset",
            [],
            None,
            "{kind: 1|2|3 (its normal view) | name: a preset (see perspective.presets.list)} reset the grid to the preset fitted to the first artboard (and show it; attached objects stay attached)",
            has_doc,
            super::perspgrid::grid_preset
        ),
        cmd!(
            "perspective.grid.show",
            "Show Perspective Grid",
            [],
            Some("Cmd+Shift+I"),
            "{visible?: bool} (no param toggles). View state: not an undo step",
            has_doc,
            grid_show
        ),
        cmd!(
            "perspective.plane.set",
            "Active Perspective Plane",
            [],
            None,
            "{plane: left|right|ground|none} Plane Switching widget. View state: not an undo step",
            has_doc,
            plane_set
        ),
        cmd!(
            "perspective.attach",
            "Attach to Active Plane",
            [],
            None,
            "{ids?, plane?: left|right|ground (default: the active plane)} project the objects onto the plane",
            has_doc,
            attach
        ),
        cmd!("perspective.release", "Release with Perspective", [], None, "{ids?} detach from the grid (geometry unchanged)", has_doc, release),
        cmd!(
            "perspective.move",
            "Move in Perspective",
            [],
            None,
            "{ids?, from: [x,y], to: [x,y], plane?, copy?: bool (move copies, as Alt-dragging does), perpendicular?: bool (move along the plane's normal instead, as pressing 5 while dragging does), snap?: bool (default: View › Perspective Grid › Snap to Grid)} slide objects within their plane (unattached objects attach to `plane`/the active plane first); snapping lands the nearer edge of the objects' joint bounds on a gridline within a quarter cell; Transform Again repeats it",
            has_doc,
            persp_move
        ),
        cmd!(
            "perspective.draw",
            "Draw in Perspective",
            [],
            None,
            "{command: shape.* id, params, plane?, snap?: bool (default: Snap to Grid)} run a shape command and attach the result to the plane (snapping its corners to gridlines within a quarter cell)",
            has_doc,
            persp_draw
        ),
        cmd!(
            "perspective.transform",
            "Transform in Perspective",
            [],
            None,
            "{ids?, matrix: [a,b,c,d,e,f] (affine map in plane coordinates, points: u along the plane, v up or away), depth?: pt to move along the plane's normal (default 0), copy?: bool, plane?} transform objects within their own planes (Perspective Selection tool handles; unattached objects attach to `plane`/the active plane first); Transform Again repeats it → {ids}",
            has_doc,
            persp_transform
        ),
        cmd!(
            "perspective.nudge",
            "Nudge in Perspective",
            [],
            None,
            "{dx, dy: arrow direction (-1, 0 or 1), big?: bool (×10), copy?: bool} move the selection in perspective by the keyboard increment, as the arrow keys do with the Perspective Selection tool → {ids}",
            has_selection,
            persp_nudge
        ),
        cmd!(
            "perspective.plane.move",
            "Move Plane",
            [],
            None,
            "{plane?: left|right|ground (default: the active plane), offset?: pt (where along its normal; 0 is its place in the grid's definition) | by?: pt, objects?: none|move|copy (default none: the objects on the plane stay; move: they move with it, as Shift-dragging a plane widget does; copy: copies of them do, as Alt-dragging does)} → {plane, offset, ids}",
            has_doc,
            plane_move
        ),
        cmd!(
            "perspective.plane.matchObject",
            "Move Plane to Match Object",
            [],
            None,
            "{id? (default: the first selected object)} move the plane the object is attached to onto the object and make it the active plane → {plane, offset}",
            has_doc,
            plane_match
        ),
        cmd!(
            "perspective.editText",
            "Edit Text",
            [],
            None,
            "{id? (default: the selected type)} Object › Perspective › Edit Text: show type in perspective flat where it is drawn, in isolation mode, to edit it (text.* commands, the Type tool); exiting isolation (object.exitIsolation, Esc) projects it again → {id}",
            has_doc,
            edit_text
        ),
    ]
}
