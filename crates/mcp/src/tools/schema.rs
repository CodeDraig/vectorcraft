//! Tool catalogue, argument schemas and capability annotations.

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::{FORMATS, OPEN_EXTS, SAVE_FORMATS, format};

// ---------- schemas ----------

fn num(desc: &str) -> Value {
    json!({"type": "number", "description": desc})
}

fn string(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}

fn paint_schema(what: &str) -> Value {
    json!({
        "description": format!("{what}: \"#rrggbb\", \"none\", [r,g,b] (0..1), {{\"c\",\"m\",\"y\",\"k\"}}, {{\"gray\"}}, or a full paint.setFill params object ({{\"gradient\":…}} / {{\"swatch\":name}})"),
        "anyOf": [{"type": "string"}, {"type": "array", "items": {"type": "number"}}, {"type": "object"}]
    })
}

fn mods_schema() -> Value {
    json!({
        "type": "object",
        "description": "Modifier keys held (cmd = Command on macOS / Ctrl elsewhere; alt = Option)",
        "properties": {"shift": {"type": "boolean"}, "alt": {"type": "boolean"}, "cmd": {"type": "boolean"}, "ctrl": {"type": "boolean"}},
        "additionalProperties": false
    })
}

fn obj(props: Value, required: &[&str]) -> Value {
    let mut o = json!({"type": "object", "properties": props, "additionalProperties": false});
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

/// The extension of each format Save writes, in [`SAVE_FORMATS`] order.
fn save_extensions() -> Vec<&'static str> {
    SAVE_FORMATS.iter().filter_map(|id| format(id)?.extensions.first().copied()).collect()
}

fn tool(name: &str, title: &str, desc: &str, schema: Value, read_only: bool) -> Value {
    let writes_file = matches!(name, "screenshot" | "save_file" | "export");
    let read_only = read_only && !writes_file;
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": schema,
        "annotations": {"title": title, "readOnlyHint": read_only, "destructiveHint": !read_only, "idempotentHint": read_only, "openWorldHint": false},
    })
}

/// All tools, in `tools/list` order.
pub fn tool_definitions() -> Vec<Value> {
    let empty = || obj(json!({}), &[]);
    vec![
        tool(
            "command_list",
            "List commands",
            "Discover command ids, labels, parameter descriptions and enabled state. Returns an array.",
            obj(json!({"filter": string("Case-insensitive id, label or menu substring"), "enabled_only": {"type":"boolean"}}), &[]),
            true,
        ),
        tool(
            "command_run",
            "Run a command",
            "Run an engine command by id with its JSON params. Command parameters follow the registry's descriptions.",
            obj(json!({"id": string("Command id"), "params": {"type":"object"}}), &["id"]),
            false,
        ),
        tool(
            "command_batch",
            "Run several commands",
            "Run steps in order; each edit is its own undo step. Returns completed, failed and results. Stops on the first error by default.",
            obj(
                json!({"steps": {"type":"array", "items":obj(json!({"id":string("Command id"),"params":{"type":"object"}}), &["id"])}, "stop_on_error":{"type":"boolean", "default":true}}),
                &["steps"],
            ),
            false,
        ),
        tool(
            "doc_inspect",
            "Inspect document",
            "Summary of the active document, including artboards, layer tree, selection and history.",
            obj(json!({"depth":{"type":"integer", "minimum":0},"childLimit":{"type":"integer", "minimum":0}}), &[]),
            true,
        ),
        tool(
            "render_preview",
            "Render a preview",
            "Render the first artboard as inline PNG without changing the document. max_side defaults to 1024, limited to 1..4096.",
            obj(json!({"max_side":{"type":"integer","minimum":1,"maximum":4096}}), &[]),
            true,
        ),
        tool("ui_inspect", "Inspect the interface", "Inspect the connected desktop interface; returns a tool error in headless mode.", empty(), true),
        tool(
            "ui_screenshot",
            "Capture the interface",
            "Capture the connected desktop window as inline PNG; requires a running desktop app.",
            empty(),
            true,
        ),
        tool(
            "select_tool",
            "Select tool",
            "Switch the active tool (as clicking it in the Tools panel). Ids: selection, directSelection, groupSelection, pen, rectangle, roundedRectangle, ellipse, polygon, star, lineSegment, …",
            obj(json!({"tool": string("Tool id")}), &["tool"]),
            false,
        ),
        tool(
            "pointer_gesture",
            "Pointer gesture",
            "Drive the active tool with mouse events in document coordinates, exactly like the mouse would (e.g. a rectangle drag: down at (100,100), drag to (200,150), up at (200,150)). Optionally selects `tool` first.",
            obj(
                json!({
                    "tool": string("Optional tool to select before the gesture"),
                    "events": {
                        "type": "array",
                        "minItems": 1,
                        "items": obj(json!({
                            "kind": {"type": "string", "enum": ["down", "drag", "up", "move", "doubleclick"]},
                            "x": num("Document x (pt)"),
                            "y": num("Document y (pt)"),
                            "mods": mods_schema(),
                            "pressure": num("Pen pressure 0..1 (default 1): the Liquify tools' intensity with Use Pressure Pen on"),
                            "holdMs": num("Milliseconds the pointer then holds still, button down (0..60000): Twirl, Pucker and Bloat keep applying"),
                        }), &["kind", "x", "y"]),
                    },
                    "mods": mods_schema(),
                }),
                &["events"],
            ),
            false,
        ),
        tool(
            "draw_path",
            "Draw path",
            "Create a path from points ([[x,y],…] or [{x,y,in?:[x,y],out?:[x,y]},…] with Bézier handles) or SVG path data `d`, then apply optional paint. Returns the new object id.",
            obj(
                json!({
                    "points": {"type": "array", "description": "Anchors: [x,y] pairs or {x, y, in?, out?, smooth?} objects", "items": {"anyOf": [{"type": "array", "items": {"type": "number"}, "minItems": 2}, {"type": "object"}]}},
                    "d": string("SVG path data (alternative to points), e.g. \"M0 0 L100 0 L50 80 Z\""),
                    "closed": {"type": "boolean", "description": "Close the path (points only)"},
                    "fill": paint_schema("Fill"),
                    "stroke": paint_schema("Stroke"),
                    "strokeWidth": num("Stroke weight in pt"),
                }),
                &[],
            ),
            false,
        ),
        tool(
            "draw_shape",
            "Draw shape",
            "Create a live shape and apply optional paint. rectangle/ellipse: x, y, width, height (rectangle: radius = corner radius). polygon: cx, cy, radius, sides. star: cx, cy, radius1, radius2, points. line: x1, y1, x2, y2. Returns the new object id.",
            obj(
                json!({
                    "shape": {"type": "string", "enum": ["rectangle", "ellipse", "polygon", "star", "line"]},
                    "x": num("Left (rectangle/ellipse)"),
                    "y": num("Top (rectangle/ellipse)"),
                    "width": num("Width (rectangle/ellipse)"),
                    "height": num("Height (rectangle/ellipse)"),
                    "radius": num("Corner radius (rectangle) or radius (polygon)"),
                    "cx": num("Centre x (polygon/star)"),
                    "cy": num("Centre y (polygon/star)"),
                    "sides": {"type": "integer", "minimum": 3, "description": "Polygon sides (default 6)"},
                    "radius1": num("Star outer radius"),
                    "radius2": num("Star inner radius"),
                    "points": {"type": "integer", "minimum": 3, "description": "Star points (default 5)"},
                    "rotation": num("Rotation in degrees (polygon/star)"),
                    "x1": num("Line start x"),
                    "y1": num("Line start y"),
                    "x2": num("Line end x"),
                    "y2": num("Line end y"),
                    "fill": paint_schema("Fill"),
                    "stroke": paint_schema("Stroke"),
                    "strokeWidth": num("Stroke weight in pt"),
                }),
                &["shape"],
            ),
            false,
        ),
        tool(
            "set_paint",
            "Set paint",
            "Set fill, stroke and/or stroke weight on the selection (or `ids`) and as the default for new art.",
            obj(
                json!({
                    "fill": paint_schema("Fill"),
                    "stroke": paint_schema("Stroke"),
                    "strokeWidth": num("Stroke weight in pt"),
                    "ids": {"type": "array", "items": {"type": "integer"}, "description": "Object ids (default: the selection)"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "press_key",
            "Press key",
            "Send a key press with modifiers, e.g. {key:\"Z\", mods:{cmd:true}} (undo), {key:\"Escape\"}, {key:\"Enter\"}. In the desktop app it goes through the real keyboard path; headless, it runs the command or tool bound to that shortcut.",
            obj(
                json!({"key": string("Key name: A–Z, 0–9, Enter, Escape, Delete, Backspace, Tab, Space, Left/Right/Up/Down, [ ]…"), "mods": mods_schema()}),
                &["key"],
            ),
            false,
        ),
        tool(
            "type_text",
            "Type text",
            "Type text into the focused widget (text tool, dialog field). Desktop app only.",
            obj(json!({"text": string("Text to type")}), &["text"]),
            false,
        ),
        tool(
            "invoke_menu",
            "Invoke menu",
            "Invoke a menu item by its command id (same ids as command_list, including UI-only commands like view.zoomIn or window.* in the desktop app).",
            obj(json!({"command": string("Menu command id"), "params": {"type": "object"}}), &["command"]),
            false,
        ),
        tool(
            "open_panel",
            "Open panel",
            "Open a panel in the desktop app's dock by id (layers, swatches, stroke, align, pathfinder, transform, …; case-insensitive, display labels like \"Layers\" work too). Desktop app only.",
            obj(json!({"panel": string("Panel id or display label")}), &["panel"]),
            false,
        ),
        tool(
            "screenshot",
            "Screenshot",
            "Render the active artboard to PNG and return it as an image (optionally also saving it to `path`). With window:true (desktop app only) captures the whole app window instead.",
            obj(
                json!({
                    "path": string("Also write the PNG here"),
                    "scale": num("Pixels per point (default 1)"),
                    "artboard": {"type": "integer", "minimum": 0, "description": "Artboard index (default 0)"},
                    "window": {"type": "boolean", "description": "Capture the app window (desktop app only)"},
                }),
                &[],
            ),
            true,
        ),
        tool(
            "open_file",
            "Open file",
            &format!(
                "Open a file as a new, active document: .{} (see command_run document.formats). Templates (.ait, native templates) open as a new untitled document. The reply's `warnings` say what didn't come in as it was (an EPS shown as its preview image says why its PostScript couldn't be read).",
                OPEN_EXTS.join(", .")
            ),
            obj(json!({"path": string("File path")}), &["path"]),
            false,
        ),
        tool(
            "save_file",
            "Save file",
            &format!(
                "Save the active document with the engine's document.save: by default to its own file in its own format (native .vectorcraft unless it was opened from or saved as SVG, PDF or a restorable .ai). With `path`, the extension picks the format: .{}. Other formats are exports (see export). The reply's `format` says what was written and `warnings` what that format loses.",
                save_extensions().join(", .")
            ),
            obj(json!({"path": string("Destination; its extension picks the format (default: the document's own file)")}), &[]),
            false,
        ),
        tool(
            "export",
            "Export",
            "Export the active document with the engine's document.export (the same bytes in the app and headless), in any format of the `format` enum: vectors (svg/svgz with the artboard as viewBox, eps, dxf, emf, wmf), pdf (one page per artboard: all, or `artboard` / `range`), rasters (one rendered artboard: png, jpg, webp, gif, png8 as an indexed .png, tiff, bmp, tga, psd with its layers), txt (the stories, back to front), vectorcraft or template (a native copy or template; the document keeps its path). command_run document.formats lists each format's options. `selection: true` exports only the selected objects, cropped to their bounds. Template layers are left out; live effects are kept (geometry baked, SVG filters for shadows/glows/blur). Without `path` the bytes come back as dataBase64.",
            obj(
                json!({
                    "format": {"type": "string", "enum": FORMATS.iter().filter(|f| f.write).map(|f| f.id).collect::<Vec<_>>(), "description": "Default: from the path's extension"},
                    "path": string("Destination file (omit to get dataBase64)"),
                    "scale": num("Raster pixels per point (default 1)"),
                    "artboard": {"type": "integer", "minimum": 0, "description": "0-based artboard (default: the first; PDF default: all)"},
                    "range": string("1-based artboards such as \"1-3, 5\" (PDF: one page each; other formats take one)"),
                    "selection": {"type": "boolean", "description": "Export only the selection, cropped to it"},
                    "outlineText": {"type": "boolean", "description": "SVG: text as outlines (viewable without the fonts)"},
                    "options": {"type": "object", "description": "More format options, passed through (see command_run document.formats), e.g. {\"quality\": 80} for jpg"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "add_text",
            "Add text",
            "Create point type at (x, y) (the first baseline), area type when width/height are given, or type in/on an existing path (`path` id + `mode` area|onPath; `pathEffect` rainbow|skew|3dRibbon|stairStep|gravity). Returns the text object id.",
            obj(
                json!({
                    "text": {"type": "string"},
                    "x": num("Baseline origin x (point/area type: left)"),
                    "y": num("First baseline y (area type: top)"),
                    "width": num("Area type frame width"),
                    "height": num("Area type frame height"),
                    "path": {"type": "integer", "description": "Existing path id to flow the text in (area) or along (onPath)"},
                    "mode": {"type": "string", "enum": ["area", "onPath"]},
                    "pathEffect": {"type": "string", "enum": ["rainbow", "skew", "3dRibbon", "stairStep", "gravity"]},
                    "size": num("Font size in pt"),
                    "font": {"type": "string", "description": "Font family"},
                    "color": paint_schema("Text colour (#rrggbb)"),
                }),
                &["text"],
            ),
            false,
        ),
        tool(
            "apply_effect",
            "Apply live effect",
            "Append a live effect (Effect menu) to the selection or `ids`. Omit `effect` to get the catalogue (ids, parameters, defaults). Examples: stylize.dropShadow {x, y, blur, opacity}, distort.roughen, warp.arc {bend}, path.offsetPath {offset}, pathfinder.add (on groups), blur.gaussian {radius}.",
            obj(
                json!({
                    "effect": {"type": "string", "description": "Effect id (see the catalogue)"},
                    "params": {"type": "object", "description": "Effect parameters; missing keys take the dialog defaults"},
                    "ids": {"type": "array", "items": {"type": "integer"}},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "pathfinder",
            "Pathfinder",
            "Combine the selected (or `ids`) objects destructively, back to front: unite, minusFront, intersect, exclude, divide, trim, merge, crop, outline, minusBack. For a non-destructive version apply the pathfinder.* effect to a group.",
            obj(
                json!({
                    "operation": {"type": "string", "enum": ["unite", "minusFront", "intersect", "exclude", "divide", "trim", "merge", "crop", "outline", "minusBack"]},
                    "ids": {"type": "array", "items": {"type": "integer"}},
                }),
                &["operation"],
            ),
            false,
        ),
        tool(
            "transform",
            "Transform",
            "Move, rotate, scale, reflect or shear the selection (or `ids`); several may be combined and run in that order. Angles in degrees (counter-clockwise), scale in %. `origin` [x,y] defaults to the selection centre; `copy` transforms a copy.",
            obj(
                json!({
                    "ids": {"type": "array", "items": {"type": "integer"}},
                    "dx": num("Move right"),
                    "dy": num("Move down"),
                    "rotate": num("Rotation in degrees"),
                    "scale": num("Uniform scale in %"),
                    "scaleX": num("Horizontal scale in %"),
                    "scaleY": num("Vertical scale in %"),
                    "reflect": {"type": "string", "enum": ["vertical", "horizontal"]},
                    "shear": num("Shear angle in degrees (horizontal axis)"),
                    "origin": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
                    "copy": {"type": "boolean"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "create_graph",
            "Create graph",
            "Create a graph (graph tools) in the plot rectangle. Data as `csv` (first row: empty cell then series names; then one row per category: label, values…) or `series`/`categories`/`rows`. Edit later with command_run graph.setData / graph.setType.",
            obj(
                json!({
                    "type": {"type": "string", "enum": ["column", "stackedColumn", "bar", "stackedBar", "line", "area", "scatter", "pie", "radar"]},
                    "x": num("Plot left"),
                    "y": num("Plot top"),
                    "width": num("Plot width"),
                    "height": num("Plot height"),
                    "csv": {"type": "string"},
                    "series": {"type": "array", "items": {"type": "string"}},
                    "categories": {"type": "array", "items": {"type": "string"}},
                    "rows": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}},
                }),
                &["x", "y", "width", "height"],
            ),
            false,
        ),
        tool(
            "text_wrap",
            "Text wrap",
            "Make the selected (or `ids`) objects wrap objects: area type below them in the same layer flows around them (offset in pt, invert = flow inside). `release: true` removes the wrap.",
            obj(
                json!({
                    "ids": {"type": "array", "items": {"type": "integer"}},
                    "offset": num("Gap between text and object (default 6 pt)"),
                    "invert": {"type": "boolean"},
                    "release": {"type": "boolean"},
                }),
                &[],
            ),
            false,
        ),
        tool("undo", "Undo", "Undo the last change (Edit → Undo).", empty(), false),
        tool("redo", "Redo", "Redo the last undone change (Edit → Redo).", empty(), false),
    ]
}
