//! Menu and context-menu composition from commands and document content.

use super::{
    Brightness, Item, PERSPECTIVE_SLOTS, RECENT_FONT_IDS, RECENT_IDS, SAVED_SELECTION_IDS, Sep, VectorcraftApp, c, cp, enabled, json,
    library_placeholders, sub, todo, todos,
};

pub fn menu_tree() -> Vec<(&'static str, Vec<Item>)> {
    let panel = |label: &'static str, id: &'static str| cp(label, "window.panel", json!({ "panel": id }));
    vec![
        (
            "VectorCraft",
            vec![
                c("About VectorCraft", "help.about"),
                c("Join Our Discord", "help.discord"),
                Sep,
                c("Settings…", "edit.preferences"),
                sub(
                    "Language",
                    std::iter::once(cp("Automatic", "app.language", json!({"lang": "auto"})))
                        .chain(std::iter::once(Sep))
                        .chain(crate::i18n::Lang::all().map(|l| cp(l.name(), "app.language", json!({"lang": l.code()}))))
                        .collect(),
                ),
                Sep,
                sub("UI Brightness", Brightness::ALL.iter().map(|b| cp(b.label(), "window.brightness", json!({"brightness": b.id()}))).collect()),
                Sep,
                c("Quit VectorCraft", "app.quit"),
            ],
        ),
        (
            "File",
            vec![
                c("New…", "file.newDialog"),
                c("New from Template…", "file.newFromTemplate"),
                c("Open…", "file.open"),
                sub("Open Recent Files", {
                    let mut v: Vec<Item> = RECENT_IDS.iter().map(|id| c("Recent File", id)).collect();
                    v.push(Sep);
                    v.push(c("Clear Recent Files", "file.clearRecent"));
                    v
                }),
                c("Show in Folder", "file.reveal"),
                Sep,
                c("Close", "file.close"),
                c("Close All", "file.closeAll"),
                c("Save", "file.save"),
                c("Save As…", "file.saveAs"),
                c("Save a Copy…", "file.saveCopy"),
                c("Save as Template…", "file.saveAsTemplate"),
                c("Save as PDF…", "file.export.pdf"),
                c("Save for Office Documents…", "document.exportForOffice"),
                c("Revert", "file.revert"),
                Sep,
                c("Place…", "file.place"),
                Sep,
                sub(
                    "Export",
                    vec![
                        c("Export for Screens…", "file.exportForScreens"),
                        c("Export As…", "file.exportAs"),
                        c("Export As SVG…", "file.export.svg"),
                        c("Export As PNG…", "file.export.png"),
                        c("Save for Web (Legacy)…", "file.saveForWeb"),
                    ],
                ),
                c("Export Selection…", "file.exportSelection"),
                Sep,
                c("Package…", "ui.packageDialog"),
                sub("Scripts", vec![todos("Other Script…", "Cmd+F12")]),
                Sep,
                c("Document Setup…", "file.documentSetup"),
                sub(
                    "Document Color Mode",
                    vec![
                        cp("CMYK Color", "file.documentColorMode", json!({"mode": "cmyk"})),
                        cp("RGB Color", "file.documentColorMode", json!({"mode": "rgb"})),
                    ],
                ),
                c("File Info…", "file.info"),
                Sep,
                c("Print…", "file.print"),
            ],
        ),
        (
            "Edit",
            vec![
                c("Undo", "edit.undo"),
                c("Redo", "edit.redo"),
                Sep,
                c("Cut", "edit.cut"),
                c("Copy", "edit.copy"),
                c("Paste", "edit.paste"),
                c("Paste in Front", "edit.pasteInFront"),
                c("Paste in Back", "edit.pasteInBack"),
                c("Paste in Place", "edit.pasteInPlace"),
                c("Paste on All Artboards", "edit.pasteOnAllArtboards"),
                c("Paste without Formatting", "edit.pasteWithoutFormatting"),
                c("Clear", "edit.clear"),
                Sep,
                cp("Find and Replace…", "edit.findReplace", json!({"find": "", "replace": "", "matchCase": false, "wholeWord": false})),
                cp("Find Next", "edit.findNext", json!({"find": ""})),
                sub("Spelling", vec![todo("Auto Spell Check"), todos("Check Spelling…", "Cmd+I"), todo("Edit Custom Dictionary…")]),
                Sep,
                sub(
                    "Edit Colors",
                    vec![
                        c("Recolor Artwork…", "ui.recolorDialog"),
                        sub(
                            "Recolor with Preset",
                            vec![
                                cp("1 Color Job…", "ui.recolorDialog", json!({"colors": 1})),
                                cp("2 Color Job…", "ui.recolorDialog", json!({"colors": 2})),
                                cp("3 Color Job…", "ui.recolorDialog", json!({"colors": 3})),
                                cp("Color Library…", "ui.recolorDialog", json!({"library": ""})),
                            ],
                        ),
                        c("Adjust Color Balance…", "ui.colorBalanceDialog"),
                        c("Blend Front to Back", "edit.colors.blendFrontToBack"),
                        c("Blend Horizontally", "edit.colors.blendHorizontally"),
                        c("Blend Vertically", "edit.colors.blendVertically"),
                        c("Convert to CMYK", "edit.colors.toCMYK"),
                        c("Convert to Grayscale", "edit.colors.toGrayscale"),
                        c("Convert to RGB", "edit.colors.toRGB"),
                        c("Invert Colors", "edit.colors.invert"),
                        cp(
                            "Overprint Black…",
                            "edit.colors.overprintBlack",
                            json!({"remove": false, "percentage": 100, "fill": true, "stroke": true, "includeCmyBlacks": false, "includeSpotBlacks": false}),
                        ),
                        c("Saturate…", "ui.saturateDialog"),
                    ],
                ),
                c("Edit Original", "links.editOriginal"),
                Sep,
                c("Transparency Flattener Presets…", "ui.flattenerPresetsDialog"),
                c("Print Presets…", "ui.printPresetsDialog"),
                c("PDF Presets…", "ui.pdfPresetsDialog"),
                c("Perspective Grid Presets…", "ui.perspectivePresetsDialog"),
                Sep,
                c("Color Settings…", "edit.colorSettings"),
                c("Assign Profile…", "edit.assignProfile"),
                Sep,
                c("Keyboard Shortcuts…", "edit.keyboardShortcuts"),
                c("Preferences…", "edit.preferences"),
            ],
        ),
        (
            "Object",
            vec![
                sub("Transform", transform_items()),
                sub("Arrange", arrange_items()),
                sub(
                    "Align",
                    vec![
                        cp("Horizontal Align Left", "object.align", json!({"horizontal": "left"})),
                        cp("Horizontal Align Center", "object.align", json!({"horizontal": "center"})),
                        cp("Horizontal Align Right", "object.align", json!({"horizontal": "right"})),
                        cp("Vertical Align Top", "object.align", json!({"vertical": "top"})),
                        cp("Vertical Align Center", "object.align", json!({"vertical": "center"})),
                        cp("Vertical Align Bottom", "object.align", json!({"vertical": "bottom"})),
                    ],
                ),
                Sep,
                c("Group", "object.group"),
                c("Ungroup", "object.ungroup"),
                sub(
                    "Lock",
                    vec![c("Selection", "object.lock"), c("All Artwork Above", "object.lock.above"), c("Other Layers", "object.lock.otherLayers")],
                ),
                c("Unlock All", "object.unlockAll"),
                sub(
                    "Hide",
                    vec![c("Selection", "object.hide"), c("All Artwork Above", "object.hide.above"), c("Other Layers", "object.hide.otherLayers")],
                ),
                c("Show All", "object.showAll"),
                Sep,
                c("Expand…", "ui.expandDialog"),
                c("Expand Appearance", "effect.expandAppearance"),
                c("Crop Image", "ui.cropImage"),
                c("Rasterize…", "object.rasterize"),
                cp("Create Gradient Mesh…", "object.mesh.create", json!({"rows": 4, "cols": 4, "appearance": "flat", "highlight": 100})),
                cp(
                    "Create Object Mosaic…",
                    "object.createObjectMosaic",
                    json!({"columns": 10, "rows": 10, "spacingX": 0, "spacingY": 0, "gray": false, "deleteRaster": false}),
                ),
                c("Vector Halftone…", "object.vectorHalftone"),
                c("Create Trim Marks", "object.createTrimMarks"),
                c("Flatten Transparency…", "ui.flattenTransparencyDialog"),
                Sep,
                c("Make Pixel Perfect", "object.makePixelPerfect"),
                Sep,
                sub(
                    "Slice",
                    vec![
                        c("Make", "object.slice.make"),
                        c("Release", "object.slice.release"),
                        c("Create from Guides", "object.slice.fromGuides"),
                        c("Create from Selection", "object.slice.fromSelection"),
                        Sep,
                        c("Duplicate Slice", "object.slice.duplicate"),
                        c("Combine Slices", "object.slice.combine"),
                        c("Divide Slices…", "object.slice.divide"),
                        Sep,
                        c("Delete All", "object.slice.deleteAll"),
                        c("Slice Options…", "object.slice.options"),
                        c("Clip to Artboard", "object.slice.clipToArtboard"),
                    ],
                ),
                Sep,
                sub(
                    "Path",
                    vec![
                        c("Join", "path.join"),
                        c("Average…", "path.average"),
                        Sep,
                        c("Outline Stroke", "object.path.outlineStroke"),
                        c("Offset Path…", "object.path.offsetPath"),
                        c("Reverse Path Direction", "path.reverse"),
                        Sep,
                        c("Simplify…", "object.path.simplify"),
                        c("Add Anchor Points", "object.path.addAnchorPoints"),
                        c("Remove Anchor Points", "path.removeAnchors"),
                        c("Divide Objects Below", "object.path.divideObjectsBelow"),
                        c("Split Into Grid…", "object.path.splitIntoGrid"),
                        Sep,
                        cp("Clean Up…", "object.path.cleanUp", json!({"strayPoints": true, "unpaintedObjects": true, "emptyTextPaths": true})),
                    ],
                ),
                sub("Shape", vec![c("Convert to Shape", "object.shape.convertToShape"), c("Expand Shape", "object.expandShape")]),
                sub(
                    "Pattern",
                    vec![c("Make", "object.pattern.make"), c("Edit Pattern", "object.pattern.edit"), c("Tile Edge Color…", "ui.tileEdgeColor")],
                ),
                sub(
                    "Repeat",
                    vec![
                        c("Radial", "object.repeat.radial"),
                        c("Grid", "object.repeat.grid"),
                        c("Mirror", "object.repeat.mirror"),
                        Sep,
                        c("Release", "object.repeat.release"),
                        c("Options…", "object.repeat.options"),
                    ],
                ),
                sub(
                    "Blend",
                    vec![
                        c("Make", "object.blend.make"),
                        c("Release", "object.blend.release"),
                        Sep,
                        c("Blend Options…", "object.blend.options"),
                        Sep,
                        c("Expand", "object.blend.expand"),
                        Sep,
                        c("Replace Spine", "object.blend.replaceSpine"),
                        c("Reverse Spine", "object.blend.reverseSpine"),
                        c("Reverse Front to Back", "object.blend.reverseFrontToBack"),
                    ],
                ),
                sub(
                    "Envelope Distort",
                    vec![
                        // While an envelope is selected, Reset takes Make's place (`swapped_out`).
                        c("Make with Warp…", "object.envelope.makeWithWarp"),
                        c("Reset with Warp…", "object.envelope.resetWithWarp"),
                        c("Make with Mesh…", "object.envelope.makeWithMesh"),
                        c("Reset with Mesh…", "object.envelope.resetWithMesh"),
                        c("Make with Top Object", "object.envelope.makeWithTopObject"),
                        Sep,
                        c("Release", "object.envelope.release"),
                        c("Envelope Options…", "object.envelope.options"),
                        c("Expand", "object.envelope.expand"),
                        Sep,
                        c("Edit Contents", "object.envelope.editContents"),
                    ],
                ),
                sub(
                    "Perspective",
                    vec![
                        c("Attach to Active Plane", "perspective.attach"),
                        c("Release with Perspective", "perspective.release"),
                        c("Move Plane to Match Object", "perspective.plane.matchObject"),
                        c("Edit Text", "perspective.editText"),
                    ],
                ),
                sub(
                    "Live Paint",
                    vec![
                        c("Make", "livePaint.make"),
                        c("Merge", "livePaint.merge"),
                        c("Release", "livePaint.release"),
                        Sep,
                        todo("Gap Options…"),
                        Sep,
                        c("Expand", "livePaint.expand"),
                    ],
                ),
                sub(
                    "Image Trace",
                    vec![
                        // Traced at once with the Default preset, as the Control bar's Image Trace
                        // button does; other presets come from its arrow or the Image Trace panel
                        // (#543: the "…" form asked for a preset's name in a text box).
                        cp("Make", "imageTrace.make", json!({"preset": "Default"})),
                        cp("Make and Expand", "imageTrace.makeAndExpand", json!({"preset": "Default"})),
                        c("Release", "imageTrace.release"),
                        c("Expand", "imageTrace.expand"),
                    ],
                ),
                sub(
                    "Text Wrap",
                    vec![
                        c("Make", "object.textWrap.make"),
                        c("Release", "object.textWrap.release"),
                        c("Text Wrap Options…", "object.textWrap.options"),
                    ],
                ),
                Sep,
                sub(
                    "Clipping Mask",
                    vec![
                        c("Make", "object.clippingMask.make"),
                        c("Release", "object.clippingMask.release"),
                        c("Edit Contents", "object.clippingMask.editContents"),
                        c("Edit Clipping Path", "object.clippingMask.editMask"),
                    ],
                ),
                sub("Compound Path", vec![c("Make", "object.compoundPath.make"), c("Release", "object.compoundPath.release")]),
                sub(
                    "Artboards",
                    vec![
                        c("Convert to Artboards", "artboard.convertToArtboards"),
                        c("Rearrange All Artboards…", "artboard.rearrange"),
                        Sep,
                        c("Fit to Artwork Bounds", "artboard.fitToArt"),
                        c("Fit to Selected Art", "artboard.fitToSelection"),
                    ],
                ),
                sub("Graph", vec![c("Type…", "graph.setType"), c("Data…", "graph.setData"), todo("Design…"), todo("Column…"), todo("Marker…")]),
                sub(
                    "Collect for Export",
                    vec![
                        cp("As Single Asset", "assets.add", json!({"multiple": false})),
                        cp("As Multiple Assets", "assets.add", json!({"multiple": true})),
                    ],
                ),
                Sep,
                sub("Plug-ins", crate::dialogs::plugin::object_menu()),
            ],
        ),
        (
            "Type",
            vec![
                sub("Font", font_items()),
                sub("Recent Fonts", RECENT_FONT_IDS.iter().map(|id| c("Recent Font", id)).collect()),
                sub("Size", TYPE_SIZES.iter().map(|(l, n)| cp(l, "text.setStyle", json!({ "size": n }))).collect()),
                c("Bold", "type.bold"),
                c("Italic", "type.italic"),
                Sep,
                panel("Glyphs", "glyphs"),
                sub("Insert Special Character", insert_items(INSERT_SPECIAL)),
                sub("Insert Whitespace Character", insert_items(INSERT_WHITESPACE)),
                sub("Insert Break Character", insert_items(INSERT_BREAK)),
                Sep,
                c("Convert To Area Type", "type.convertToAreaType"),
                c("Convert To Point Type", "type.convertToPointType"),
                c("Area Type Options…", "text.areaOptions"),
                sub(
                    "Type on a Path",
                    PATH_EFFECTS
                        .iter()
                        .map(|&(label, effect)| cp(label, "type.pathOptions", json!({ "effect": effect })))
                        .chain([Sep, c("Type on a Path Options…", "type.pathOptions")])
                        .collect(),
                ),
                sub(
                    "Threaded Text",
                    vec![
                        c("Create", "text.thread.create"),
                        c("Release Selection", "text.thread.releaseSelection"),
                        c("Remove Threading", "text.thread.remove"),
                    ],
                ),
                c("Fit Headline", "text.fitHeadline"),
                Sep,
                c("Find Font…", "type.findFont"),
                sub(
                    "Change Case",
                    vec![
                        cp("UPPERCASE", "type.changeCase", json!({"case": "upper"})),
                        cp("lowercase", "type.changeCase", json!({"case": "lower"})),
                        cp("Title Case", "type.changeCase", json!({"case": "title"})),
                        cp("Sentence case", "type.changeCase", json!({"case": "sentence"})),
                    ],
                ),
                cp("Smart Punctuation…", "type.smartPunctuation", json!({"quotes": true, "dashes": true, "ellipsis": true, "scope": "selection"})),
                Sep,
                c("Create Outlines", "type.createOutlines"),
                todo("Optical Margin Alignment"),
                Sep,
                c("Fill with Placeholder Text", "type.fillPlaceholder"),
                c("Insert Inline Symbol", "text.insertInline"),
                Sep,
                c("Show Hidden Characters", "type.hiddenCharacters"),
                sub("Type Orientation", vec![c("Horizontal", "type.orientation.horizontal"), c("Vertical", "type.orientation.vertical")]),
            ],
        ),
        ("Select", {
            let mut v = vec![
                c("All", "select.all"),
                c("All on Active Artboard", "select.allOnArtboard"),
                c("Deselect", "select.none"),
                c("Reselect", "select.reselect"),
                Sep,
                c("Inverse", "select.inverse"),
                Sep,
                c("Next Object Above", "select.nextAbove"),
                c("Next Object Below", "select.nextBelow"),
                Sep,
                sub(
                    "Same",
                    vec![
                        c("Appearance", "select.same.appearance"),
                        c("Appearance Attribute", "select.same.appearanceAttribute"),
                        c("Blending Mode", "select.same.blendingMode"),
                        c("Fill & Stroke", "select.same.fillAndStroke"),
                        c("Fill Color", "select.same.fillColor"),
                        c("Opacity", "select.same.opacity"),
                        c("Stroke Color", "select.same.strokeColor"),
                        c("Stroke Weight", "select.same.strokeWeight"),
                        c("Graphic Style", "select.same.graphicStyle"),
                        c("Shape", "select.same.shapeType"),
                        c("Symbol Instance", "select.same.symbolInstance"),
                        Sep,
                        c("Font Family", "select.same.fontFamily"),
                        c("Font Family & Style", "select.same.fontFamilyStyle"),
                        c("Font Family, Style & Size", "select.same.fontFamilyStyleSize"),
                        c("Font Size", "select.same.fontSize"),
                        c("Text Fill Color", "select.same.textFillColor"),
                        c("Text Stroke Color", "select.same.textStrokeColor"),
                    ],
                ),
                sub(
                    "Object",
                    vec![
                        c("All on Same Layers", "select.object.allOnSameLayers"),
                        c("Direction Handles", "select.object.directionHandles"),
                        Sep,
                        c("Brush Strokes", "select.object.brushStrokes"),
                        c("Bristle Brush Strokes", "select.object.bristleBrushStrokes"),
                        c("Clipping Masks", "select.object.clippingMasks"),
                        c("Slices", "select.object.slices"),
                        c("Stray Points", "select.object.strayPoints"),
                        c("Open Paths", "select.object.openPaths"),
                        Sep,
                        c("All Text Objects", "select.object.textObjects"),
                        c("Point Text Objects", "select.object.pointText"),
                        c("Area Text Objects", "select.object.areaText"),
                    ],
                ),
                todo("Start Global Edit"),
                Sep,
                c("Save Selection…", "select.save"),
                c("Edit Selection…", "select.editSaved"),
                Sep,
            ];
            v.extend(SAVED_SELECTION_IDS.iter().map(|id| c("Saved Selection", id)));
            v
        }),
        ("Effect", effect_menu()),
        (
            "View",
            vec![
                c("Outline", "view.outline"),
                c("Overprint Preview", "view.overprintPreview"),
                c("Pixel Preview", "view.pixelPreview"),
                c("Trim View", "view.trimView"),
                c("Presentation Mode", "view.presentation"),
                sub(
                    "Screen Mode",
                    vec![
                        cp("Normal Screen Mode", "view.screenMode", json!({"mode": 0})),
                        cp("Full Screen Mode with Menu Bar", "view.screenMode", json!({"mode": 1})),
                        cp("Full Screen Mode", "view.screenMode", json!({"mode": 2})),
                    ],
                ),
                Sep,
                sub(
                    "Proof Setup",
                    vec![
                        cp("Working CMYK", "view.proofSetup", json!({"target": "workingCmyk", "proof": true})),
                        cp("Legacy Macintosh RGB (Gamma 1.8)", "view.proofSetup", json!({"target": "legacyMacRgb", "proof": true})),
                        cp("Internet Standard RGB (sRGB)", "view.proofSetup", json!({"target": "srgb", "proof": true})),
                        cp("Monitor RGB", "view.proofSetup", json!({"target": "monitorRgb", "proof": true})),
                        cp("Color blindness – Protanopia-type", "view.proofSetup", json!({"target": "protanopia", "proof": true})),
                        cp("Color blindness – Deuteranopia-type", "view.proofSetup", json!({"target": "deuteranopia", "proof": true})),
                        cp("Customize…", "window.panel", json!({"panel": "separations"})),
                    ],
                ),
                c("Proof Colors", "view.proofColors"),
                Sep,
                c("Zoom In", "view.zoomIn"),
                c("Zoom Out", "view.zoomOut"),
                c("Fit Artboard in Window", "view.fitArtboard"),
                c("Fit All in Window", "view.fitAll"),
                c("Actual Size", "view.actualSize"),
                Sep,
                c("Reset Rotate View", "view.rotateReset"),
                Sep,
                c("Hide Edges", "view.edges"),
                c("Hide Artboards", "view.artboards"),
                c("Show Print Tiling", "view.printTiling"),
                c("Hide Slices", "view.slices.hide"),
                c("Lock Slices", "view.slices.lock"),
                Sep,
                sub("Rulers", vec![c("Show Rulers", "view.rulers"), todos("Change to Global Rulers", "Cmd+Alt+R"), todo("Show Video Rulers")]),
                c("Hide Bounding Box", "view.boundingBox"),
                c("Show Transparency Grid", "view.transparencyGrid"),
                c("Hide Text Threads", "view.textThreads"),
                c("Hide Gradient Annotator", "view.gradientAnnotator"),
                c("Hide Corner Widget", "view.cornerWidget"),
                Sep,
                sub(
                    "Guides",
                    vec![
                        c("Hide Guides", "view.guides"),
                        c("Lock Guides", "view.guides.lock"),
                        c("Make Guides", "view.guides.make"),
                        c("Release Guides", "view.guides.release"),
                        c("Clear Guides", "view.guides.clear"),
                    ],
                ),
                c("Smart Guides", "view.smartGuides"),
                sub(
                    "Perspective Grid",
                    vec![
                        c("Show Grid", "perspective.grid.show"),
                        c("Show Rulers", "perspective.grid.rulers"),
                        c("Snap to Grid", "perspective.grid.snap"),
                        c("Lock Grid", "perspective.grid.lock"),
                        c("Lock Station Point", "perspective.grid.lockStation"),
                        Sep,
                        c("Define Grid…", "ui.perspectiveGridDialog"),
                        sub("One Point Perspective", crate::dialogs::perspective_presets::menu(1, PERSPECTIVE_SLOTS[0])),
                        sub("Two Point Perspective", crate::dialogs::perspective_presets::menu(2, PERSPECTIVE_SLOTS[1])),
                        sub("Three Point Perspective", crate::dialogs::perspective_presets::menu(3, PERSPECTIVE_SLOTS[2])),
                        Sep,
                        c("Save Grid as Preset…", "ui.savePerspectivePreset"),
                    ],
                ),
                c("Show Grid", "view.grid"),
                c("Snap to Grid", "view.snapToGrid"),
                c("Snap to Pixel", "view.snapToPixel"),
                c("Snap to Point", "view.snapToPoint"),
                todo("Snap to Glyph"),
                Sep,
                c("New View…", "view.saved.new"),
                c("Edit Views…", "view.saved.edit"),
                Sep,
                c("Saved View 1", "view.goto1"),
                c("Saved View 2", "view.goto2"),
                c("Saved View 3", "view.goto3"),
                c("Saved View 4", "view.goto4"),
                c("Saved View 5", "view.goto5"),
                c("Saved View 6", "view.goto6"),
                c("Saved View 7", "view.goto7"),
                c("Saved View 8", "view.goto8"),
                c("Saved View 9", "view.goto9"),
                c("Saved View 10", "view.goto10"),
            ],
        ),
        (
            "Window",
            vec![
                c("New Window", "window.newWindow"),
                sub("Arrange", vec![todo("Cascade"), todo("Tile"), todo("Float in Window"), todo("Consolidate All Windows")]),
                sub("Workspace", crate::workspaces::menu_items()),
                Sep,
                c("Control", "window.control"),
                c("Contextual Task Bar", "window.taskBar"),
                c("Tools", "window.toolbar"),
                sub("Toolbars", vec![c("Advanced", "window.toolbarAdvanced"), c("Double Column", "window.toolbarColumns")]),
                Sep,
                panel("Actions", "actions"),
                panel("Align", "align"),
                panel("Appearance", "appearance"),
                panel("Artboards", "artboards"),
                panel("Asset Export", crate::panels::asset_export::ID),
                panel("Attributes", "attributes"),
                panel("Brushes", "brushes"),
                panel("Color", "color"),
                panel("Color Guide", "colorGuide"),
                panel("Color Themes", "colorThemes"),
                panel("CSS Properties", crate::panels::css_properties::ID),
                panel("Document Info", "docInfo"),
                panel("Flattener Preview", crate::panels::flattener_preview::ID),
                panel("Gradient", "gradient"),
                panel("Graphic Styles", "graphicStyles"),
                panel("History", "history"),
                panel("Image Trace", "imageTrace"),
                panel("Info", "info"),
                panel("Layers", "layers"),
                panel("Libraries", "libraries"),
                panel("Links", crate::panels::links::ID),
                panel("Magic Wand", "magicWand"),
                panel("Navigator", "navigator"),
                panel("Pathfinder", "pathfinder"),
                panel("Pattern Options", "patternOptions"),
                panel("Properties", "properties"),
                panel("Separations Preview", "separations"),
                panel("Stroke", "stroke"),
                todo("SVG Interactivity"),
                panel("Swatches", "swatches"),
                panel("Symbols", "symbols"),
                panel("Transform", "transform"),
                panel("Transparency", "transparency"),
                sub(
                    "Type",
                    vec![
                        panel("Character", "character"),
                        panel("Character Styles", "charStyles"),
                        panel("Glyphs", "glyphs"),
                        panel("OpenType", "openType"),
                        panel("Paragraph", "paragraph"),
                        panel("Paragraph Styles", "paraStyles"),
                        panel("Tabs", "tabs"),
                    ],
                ),
                todo("Variables"),
                Sep,
                sub("Brush Libraries", library_placeholders()),
                sub("Graphic Style Libraries", crate::panels::graphic_styles::window_menu()),
                sub("Swatch Libraries", crate::panels::swatches::window_menu()),
                sub("Symbol Libraries", library_placeholders()),
            ],
        ),
        (
            "Help",
            vec![
                c("Join Our Discord", "help.discord"),
                c("ArtCraft Website", "help.website"),
                c("VectorCraft on getartcraft.com", "help.appPage"),
                c("VectorCraft on GitHub", "help.github"),
                Sep,
                c("Search Commands…", "help.commandPalette"),
                todos("VectorCraft Help…", "F1"),
                Sep,
                c("About VectorCraft", "help.about"),
            ],
        ),
    ]
}

/// Object → Transform (also in the canvas context menu).
fn transform_items() -> Vec<Item> {
    vec![
        c("Transform Again", "object.transformAgain"),
        Sep,
        c("Move…", "object.move"),
        c("Rotate…", "object.rotate"),
        c("Reflect…", "object.reflect"),
        c("Scale…", "object.scale"),
        c("Shear…", "object.shear"),
        Sep,
        c("Transform Each…", "object.transformEach"),
        Sep,
        c("Reset Bounding Box", "object.resetBoundingBox"),
    ]
}

/// Object → Arrange (also in the canvas context menu).
fn arrange_items() -> Vec<Item> {
    vec![
        c("Bring to Front", "object.arrange.bringToFront"),
        c("Bring Forward", "object.arrange.bringForward"),
        c("Send Backward", "object.arrange.sendBackward"),
        c("Send to Back", "object.arrange.sendToBack"),
        Sep,
        c("Send to Current Layer", "object.arrange.sendToCurrentLayer"),
    ]
}

/// The canvas context menu (right-click): what applies to the selection, or to the view when
/// nothing is selected. Commands that can't run now are left out rather than greyed out.
pub fn context_items(app: &VectorcraftApp) -> Vec<Item> {
    use vectorcraft_doc::NodeKind;
    let Some(st) = app.session.active() else { return vec![] };
    let roots: Vec<&vectorcraft_doc::Node> = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).collect();
    let any = |f: fn(&NodeKind) -> bool| roots.iter().any(|n| f(&n.kind));
    let several = roots.len() >= 2;
    let mut v = vec![c("Undo", "edit.undo"), c("Redo", "edit.redo"), Sep];
    if st.isolation.is_some() {
        v.extend([c("Exit Isolation Mode", "object.exitIsolation"), Sep]);
    }
    if roots.is_empty() {
        v.extend([
            c("Paste", "edit.paste"),
            Sep,
            c("Zoom In", "view.zoomIn"),
            c("Zoom Out", "view.zoomOut"),
            c("Fit Artboard in Window", "view.fitArtboard"),
            Sep,
            c("Show Rulers", "view.rulers"),
            c("Show Grid", "view.grid"),
            c("Hide Guides", "view.guides"),
            c("Lock Guides", "view.guides.lock"),
            Sep,
            c("Select All", "select.all"),
        ]);
    } else {
        v.extend([c("Cut", "edit.cut"), c("Copy", "edit.copy"), c("Paste", "edit.paste"), Sep]);
        if let [one] = roots.as_slice()
            && one.is_container()
        {
            v.push(c("Isolate Selected Group", "object.isolate"));
        }
        if several {
            v.push(c("Group", "object.group"));
        }
        if any(|k| matches!(k, NodeKind::Group { clip: false, .. })) {
            v.push(c("Ungroup", "object.ungroup"));
        }
        let paths = any(|k| matches!(k, NodeKind::Path { .. }));
        if paths {
            v.extend([c("Join", "path.join"), c("Average…", "path.average")]);
        }
        if !st.selection.anchors.is_empty() {
            v.extend([c("Remove Anchor Points", "path.removeAnchors"), c("Cut Path at Selected Anchor Points", "path.cutAtAnchors")]);
        }
        if several {
            v.push(c("Make Clipping Mask", "object.clippingMask.make"));
        }
        if any(|k| matches!(k, NodeKind::Group { clip: true, .. })) {
            v.push(c("Release Clipping Mask", "object.clippingMask.release"));
        }
        if several && paths {
            v.push(c("Make Compound Path", "object.compoundPath.make"));
        }
        if any(|k| matches!(k, NodeKind::Compound { .. })) {
            v.push(c("Release Compound Path", "object.compoundPath.release"));
        }
        v.extend([
            c("Make Guides", "view.guides.make"),
            Sep,
            sub("Transform", transform_items()),
            sub("Arrange", arrange_items()),
            sub(
                "Select",
                vec![c("Next Object Above", "select.nextAbove"), c("Next Object Below", "select.nextBelow"), Sep, c("Deselect", "select.none")],
            ),
            Sep,
            c("Export Selection…", "file.exportSelection"),
        ]);
    }
    prune(app, v)
}

/// `items` without the commands that can't run now (Undo and Redo stay, greyed out), submenus
/// left empty, and separators that no longer separate anything.
fn prune(app: &VectorcraftApp, items: Vec<Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    for it in items {
        let it = match it {
            Item::Sub(l, children) => Item::Sub(l, prune(app, children)),
            it => it,
        };
        let keep = match &it {
            Item::Cmd(_, id, _) => matches!(*id, "edit.undo" | "edit.redo") || enabled(app, id),
            Item::Sub(_, children) => !children.is_empty(),
            Item::Sep => out.last().is_some_and(|l| !matches!(l, Item::Sep)),
            _ => true,
        };
        if keep {
            out.push(it);
        }
    }
    if matches!(out.last(), Some(Item::Sep)) {
        out.pop();
    }
    out
}

/// The document units a right-click on a ruler offers: each runs `document.setup`, exactly as
/// Preferences ▸ Units ▸ General does, with the current one checked ([`checked`]).
pub fn ruler_unit_items() -> Vec<Item> {
    vectorcraft_engine::doc::Unit::ALL.iter().map(|u| cp(u.label(), "document.setup", json!({ "units": u.label() }))).collect()
}

/// Type → Size presets.
const TYPE_SIZES: [(&str, u32); 14] = [
    ("6 pt", 6),
    ("8 pt", 8),
    ("9 pt", 9),
    ("10 pt", 10),
    ("11 pt", 11),
    ("12 pt", 12),
    ("14 pt", 14),
    ("18 pt", 18),
    ("24 pt", 24),
    ("30 pt", 30),
    ("36 pt", 36),
    ("48 pt", 48),
    ("60 pt", 60),
    ("72 pt", 72),
];

/// Type → Type on a Path effects (label, `type.pathOptions` effect), also the choices of its
/// Options dialog.
pub(crate) const PATH_EFFECTS: &[(&str, &str)] =
    &[("Rainbow", "rainbow"), ("Skew", "skew"), ("3D Ribbon", "3dRibbon"), ("Stair Step", "stairStep"), ("Gravity", "gravity")];

const INSERT_SPECIAL: &[(&str, &str)] = &[
    ("Bullet", "bullet"),
    ("Copyright Symbol", "copyright"),
    ("Ellipsis", "ellipsis"),
    ("Paragraph Symbol", "paragraph"),
    ("Registered Trademark Symbol", "registered"),
    ("Section Symbol", "section"),
    ("Trademark Symbol", "trademark"),
    ("Em Dash", "emDash"),
    ("En Dash", "enDash"),
    ("Discretionary Hyphen", "discretionaryHyphen"),
    ("Nonbreaking Hyphen", "nonBreakingHyphen"),
    ("Double Left Quotation Marks", "doubleLeftQuote"),
    ("Double Right Quotation Marks", "doubleRightQuote"),
    ("Single Left Quotation Mark", "singleLeftQuote"),
    ("Single Right Quotation Mark", "singleRightQuote"),
];

const INSERT_WHITESPACE: &[(&str, &str)] = &[
    ("Em Space", "emSpace"),
    ("En Space", "enSpace"),
    ("Hair Space", "hairSpace"),
    ("Sixth Space", "sixthSpace"),
    ("Thin Space", "thinSpace"),
    ("Nonbreaking Space", "nonBreakingSpace"),
    ("Figure Space", "figureSpace"),
    ("Punctuation Space", "punctuationSpace"),
    ("Third Space", "thirdSpace"),
    ("Quarter Space", "quarterSpace"),
];

const INSERT_BREAK: &[(&str, &str)] = &[("Forced Line Break", "forcedLineBreak"), ("Tab", "tab")];

fn insert_items(list: &[(&'static str, &'static str)]) -> Vec<Item> {
    list.iter().map(|(l, c)| cp(l, "type.insert", json!({ "char": c }))).collect()
}

/// Type → Font: one item per available family, the installed fonts included. Built again only when
/// the fonts change (the menu tree is built every frame); labels are interned once each, so
/// rebuilding doesn't allocate forever.
fn font_items() -> Vec<Item> {
    static NAMES: std::sync::Mutex<std::collections::BTreeSet<&'static str>> = std::sync::Mutex::new(std::collections::BTreeSet::new());
    static ITEMS: std::sync::Mutex<(u64, Vec<Item>)> = std::sync::Mutex::new((u64::MAX, Vec::new()));
    let db = vectorcraft_text::FontDb::global();
    // Read first: fonts that load meanwhile make the next frame build the list again.
    let generation = db.generation();
    let fams = db.menu_family_list();
    let (Ok(mut names), Ok(mut items)) = (NAMES.lock(), ITEMS.lock()) else { return vec![] };
    if items.0 != generation {
        let list = fams
            .iter()
            .map(|f| {
                let label: &'static str = match names.get(f.as_str()) {
                    Some(n) => n,
                    None => {
                        let n: &'static str = Box::leak(f.clone().into_boxed_str());
                        names.insert(n);
                        n
                    }
                };
                cp(label, "text.setStyle", json!({ "font": label }))
            })
            .collect();
        *items = (generation, list);
    }
    items.1.clone()
}

/// The raster effects' submenus of the Effect menu (the Photoshop-style effects, below the
/// vector effects): (submenu, the catalogue's menu path of its effects).
const RASTER_MENUS: [(&str, &[&str]); 2] = [("Blur", &["Effect", "Blur"]), ("Sharpen", &["Effect", "Sharpen"])];

/// The Effect menu, built from the effects catalogue (vector effects), plus raster effects.
fn effect_menu() -> Vec<Item> {
    let cat = vectorcraft_effects::effect_catalog();
    let mut out = vec![
        c("Apply Last Effect", "effect.applyLast"),
        c("Last Effect…", "effect.last"),
        Sep,
        c("Document Raster Effects Settings…", "document.rasterEffectsSettings"),
        Sep,
        Item::Header("Vector Effects"),
    ];
    // Submenus (and Crop Marks, an item of its own) in the reference app's order.
    let order = [
        "3D and Materials",
        "Color Adjustments",
        "Convert to Shape",
        "Crop Marks",
        "Distort & Transform",
        "Path",
        "Pathfinder",
        "Stylize",
        "SVG Filters",
        "Warp",
    ];
    let top_level = |e: &vectorcraft_effects::EffectInfo| e.menu == ["Effect"] && order.contains(&e.label);
    // The effects whose catalogue menu path is `path`.
    let items_at = |path: &[&str]| -> Vec<Item> {
        cat.iter()
            .filter(|e| e.menu == path)
            .map(|e| match e.defaults.as_object().is_some_and(|o| o.is_empty()) {
                // No options (Effect → Pathfinder): apply directly, like Illustrator.
                true => Item::Cmd(e.label, "effect.apply", json!({ "effect": e.id })),
                false => Item::Cmd(e.label, "effect.dialog", json!({ "effect": e.id })),
            })
            .collect()
    };
    for sub_name in order {
        if let Some(e) = cat.iter().find(|e| top_level(e) && e.label == sub_name) {
            out.push(Item::Cmd(e.label, "effect.apply", json!({ "effect": e.id })));
            continue;
        }
        let items = items_at(&["Effect", sub_name]);
        if items.is_empty() {
            let placeholder = match sub_name {
                "3D and Materials" => vec![todo("Extrude & Bevel…"), todo("Revolve…"), todo("Inflate…"), todo("Rotate…"), todo("Materials…")],
                "SVG Filters" => vec![todo("Apply SVG Filter…")],
                _ => continue,
            };
            out.push(sub(sub_name, placeholder));
        } else {
            out.push(sub(sub_name, items));
        }
    }
    // Live-effect plug-ins close the vector effects.
    out.extend(crate::dialogs::plugin::effect_menu());
    let raster: Vec<Item> = RASTER_MENUS
        .iter()
        .filter_map(|(name, path)| {
            let items = items_at(path);
            (!items.is_empty()).then(|| sub(name, items))
        })
        .collect();
    if !raster.is_empty() {
        out.push(Sep);
        out.push(Item::Header("Raster Effects"));
        out.extend(raster);
    }
    // Anything not placed above (future effects) still shows up.
    let placed = |e: &vectorcraft_effects::EffectInfo| {
        top_level(e) || order.iter().any(|s| e.menu == ["Effect", *s]) || RASTER_MENUS.iter().any(|(_, path)| e.menu == *path)
    };
    for e in cat.iter().filter(|e| !placed(e)) {
        out.push(Item::Cmd(e.label, "effect.dialog", json!({ "effect": e.id })));
    }
    out
}
