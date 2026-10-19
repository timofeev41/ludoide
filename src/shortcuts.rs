//! The keyboard shortcuts, listed in Settings → Shortcuts and on the welcome
//! screen. Keep in sync with `App::global_shortcuts` and `editor::view`.

pub struct Group {
    pub name: &'static str,
    pub items: &'static [(&'static str, &'static str)],
}

pub const GROUPS: &[Group] = &[
    Group {
        name: "General",
        items: &[
            ("⌘F / ⌘⇧F", "Find in project (file names and contents)"),
            ("⌘N", "New SQL scratch"),
            ("⌘S", "Save"),
            ("⌘W", "Close tab"),
            ("⌘1 … ⌘0", "Go to tab 1–10"),
            ("Ctrl+Tab / Ctrl+⇧Tab", "Next / previous tab"),
            ("⌘⇧] / ⌘⇧[", "Next / previous tab"),
            ("⌘,", "Settings"),
            ("⌘+ / ⌘−", "Zoom in / out (10% steps)"),
        ],
    },
    Group {
        name: "Panels",
        items: &[
            ("⌘B", "Toggle the left panel"),
            ("⌘J", "Toggle the bottom panel"),
            ("⌘⇧D", "Toggle the Databases panel"),
            ("⌘⇧G", "Git"),
            ("⌘⇧`", "New terminal"),
        ],
    },
    Group {
        name: "Editing",
        items: &[
            ("⌘R", "Find / replace in the current file"),
            ("↵ / ⇧↵", "Next / previous match (in Find)"),
            ("⌘] / Tab", "Indent line(s)"),
            ("⌘[ / ⇧Tab", "Outdent line(s)"),
            ("⌘/", "Toggle line comment"),
            ("⌥⇧↑ / ⌥⇧↓", "Move line(s) up / down"),
            ("⇧ + arrows", "Extend the selection"),
            ("⌘A / ⌘C / ⌘X / ⌘V", "Select all / copy / cut / paste"),
            ("⌘Z / ⌘⇧Z", "Undo / redo"),
            ("( [ { ' \" `", "With a selection: wrap it"),
        ],
    },
    Group {
        name: "SQL & dbt",
        items: &[
            ("⌘↵", "Run the statement at the cursor (or the selection)"),
            ("⌘⇧↵", "Run the whole script"),
            ("⌘ + click", "On ref('model'): open the model's file"),
        ],
    },
    Group { name: "Git", items: &[("⌘↵", "In the commit message: commit")] },
    Group {
        name: "Terminal",
        items: &[
            ("Drag / double-click", "Select text / a word"),
            ("⌘C", "Copy the selection (right-click for more)"),
            ("Double-click a tab", "Rename the terminal"),
        ],
    },
];
