use super::file_tree::{File, FileTree, FileType};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone, PartialEq)]
enum PrefixSegment {
    ShapeL, // "└── "
    ShapeT, // "├── "
    ShapeI, // "│   "
    Empty,  // "    "
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormattedEntry {
    pub name: String,
    pub path: String,
    pub prefix: String,
    pub link: Option<String>,
    /// Whether this entry represents a synthetic summary rather than a real filesystem path.
    /// Used by alias-generation to avoid producing broken aliases.
    pub virtual_entry: bool,
}

fn make_prefix(tree: &FileTree, file: &File, format_history: &HashMap<usize, usize>) -> String {
    let mut segments = Vec::new();
    let mut current = file;
    if let Some(ancestor) = tree.get_parent(file) {
        let count = format_history.get(&ancestor.id).unwrap_or(&0);
        if *count >= ancestor.children_count() - 1 {
            segments.push(PrefixSegment::ShapeL);
        } else {
            segments.push(PrefixSegment::ShapeT);
        }
        current = ancestor;
    }

    while let Some(ancestor) = tree.get_parent(current) {
        let count = format_history.get(&ancestor.id).unwrap_or(&0);
        if *count == ancestor.children_count() {
            segments.push(PrefixSegment::Empty);
        } else {
            segments.push(PrefixSegment::ShapeI);
        }
        current = ancestor;
    }

    segments.reverse();
    segments.iter().fold(String::new(), |s, seg| {
        s + match seg {
            PrefixSegment::ShapeL => "└── ",
            PrefixSegment::ShapeT => "├── ",
            PrefixSegment::ShapeI => "│   ",
            PrefixSegment::Empty => "    ",
        }
    })
}

fn format_file(
    tree: &FileTree,
    file: &File,
    format_history: &mut HashMap<usize, usize>,
    result: &mut Vec<FormattedEntry>,
    make_absolute: bool,
) {
    let prefix = make_prefix(tree, file, format_history);
    let path = if make_absolute {
        fs::canonicalize(&file.path).unwrap().display().to_string()
    } else {
        file.path.clone()
    };

    result.push(FormattedEntry {
        name: file.display_name.clone(),
        path,
        prefix,
        link: file.link(),
        virtual_entry: false,
    });

    if let Some(parent) = tree.get_parent(file) {
        if let Some(&n) = format_history.get(&parent.id) {
            format_history.insert(parent.id, n + 1);
        }
    }

    if let FileType::Directory = file.file_type {
        format_history.insert(file.id, 0);
    }

    if let Some(children) = file.children() {
        for child_id in children.values() {
            format_file(
                tree,
                tree.get(*child_id),
                format_history,
                result,
                make_absolute,
            );
        }
    }
}

pub fn format_paths(
    root_path: &str,
    children: Vec<(String, FileType)>,
    make_absolute: bool,
    max_lines: Option<usize>,
) -> Vec<FormattedEntry> {
    match FileTree::new(root_path, children) {
        Some(mut tree) => {
            if max_lines.is_none() {
                let mut history = HashMap::new();
                let mut result = Vec::new();
                let root = tree.get_root();
                format_file(&tree, root, &mut history, &mut result, make_absolute);
                return result;
            }

            tree.compute_metadata();
        let planner = SmartPlanner::new(tree, make_absolute, max_lines.unwrap());
        planner.plan()
        }
        None => Vec::new(),
    }
}

#[derive(Clone)]
struct ChildPick {
    id: usize,
    name: String,
    score: f64,
}

struct SmartPlanner {
    tree: FileTree,
    make_absolute: bool,
    result: Vec<FormattedEntry>,
    max_lines: usize,
}

impl SmartPlanner {
    fn new(tree: FileTree, make_absolute: bool, max_lines: usize) -> Self {
        SmartPlanner {
            tree,
            make_absolute,
            result: Vec::new(),
            max_lines,
        }
    }

    fn plan(mut self) -> Vec<FormattedEntry> {
        if self.max_lines == 0 {
            return Vec::new();
        }

        let root_id = self.tree.root_id;
        let mut prefix_stack: Vec<PrefixSegment> = Vec::new();
        self.render_node(root_id, &mut prefix_stack, self.max_lines, true);
        self.result
    }

    fn render_prefix(prefix_stack: &[PrefixSegment]) -> String {
        prefix_stack.iter().fold(String::new(), |mut acc, seg| {
            acc.push_str(match seg {
                PrefixSegment::ShapeL => "└── ",
                PrefixSegment::ShapeT => "├── ",
                PrefixSegment::ShapeI => "│   ",
                PrefixSegment::Empty => "    ",
            });
            acc
        })
    }

    fn push_entry(
        &mut self,
        name: String,
        path: String,
        prefix_stack: &[PrefixSegment],
        link: Option<String>,
        virtual_entry: bool,
    ) {
        self.result.push(FormattedEntry {
            name,
            path,
            prefix: Self::render_prefix(prefix_stack),
            link,
            virtual_entry,
        });
    }

    fn child_score(&self, id: usize) -> f64 {
        let file = self.tree.get(id);
        let base = (file.total_descendants as f64 + 1.0).ln();
        let branching = (file.child_dir_count as f64) * 1.2 + (file.child_file_count as f64) * 0.3;
        let skinny_bonus = if file.is_skinny() { 1.5 } else { 0.0 };
        let file_bonus = match file.file_type {
            FileType::File | FileType::Link => 0.2,
            FileType::Directory => 0.8,
        };
        base + branching + skinny_bonus + file_bonus
    }

    fn render_node(
        &mut self,
        node_id: usize,
        prefix_stack: &mut Vec<PrefixSegment>,
        budget: usize,
        force_show_children: bool,
    ) -> usize {
        if budget == 0 {
            return 0;
        }

        let mut name_parts: Vec<String> = Vec::new();
        let mut current = node_id;
        let mut collapsed_depth = 0;

        if self.tree.get(current).is_skinny() {
            while self.tree.get(current).is_skinny() {
                let node = self.tree.get(current);
                name_parts.push(node.display_name.clone());
                collapsed_depth += 1;
                if let Some(children) = node.children() {
                    current = *children.values().next().unwrap();
                } else {
                    break;
                }
                if !self.tree.get(current).is_skinny() {
                    name_parts.push(self.tree.get(current).display_name.clone());
                    break;
                }
            }
        } else {
            name_parts.push(self.tree.get(current).display_name.clone());
        }

        let display_name = if name_parts.len() > 1 {
            format!("{}/ (skinny x{})", name_parts.join("/"), collapsed_depth)
        } else {
            name_parts.join("")
        };

        let node_for_children = current;
        let path = if self.make_absolute {
            fs::canonicalize(&self.tree.get(node_for_children).path)
                .unwrap_or_else(|_| self.tree.get(node_for_children).path.clone().into())
                .display()
                .to_string()
        } else {
            self.tree.get(node_for_children).path.clone()
        };

        let mut display_name = display_name;
        if budget == 1 {
            let child_counts = self.tree.get(node_for_children).child_dir_count
                + self.tree.get(node_for_children).child_file_count;
            if child_counts > 0 {
                display_name = format!("{} … ({} hidden)", display_name, child_counts);
            }
        }

        self.push_entry(
            display_name,
            path,
            prefix_stack,
            self.tree.get(node_for_children).link(),
            false,
        );

        if budget == 1 {
            return 1;
        }

        let children = match self.tree.get(node_for_children).children() {
            Some(c) => c.clone(),
            None => return 1,
        };

        let mut child_entries: Vec<(String, usize)> = children
            .iter()
            .map(|(name, id)| (name.clone(), *id))
            .collect();
        child_entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut picks: Vec<ChildPick> = child_entries
            .iter()
            .map(|(name, id)| ChildPick {
                id: *id,
                name: name.clone(),
                score: self.child_score(*id),
            })
            .collect();

        picks.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap()
                .then_with(|| a.name.cmp(&b.name))
        });

        let mut visible_children: Vec<usize> = Vec::new();
        let available_for_children = budget - 1;

        if force_show_children || picks.len() <= available_for_children {
            visible_children = picks.iter().map(|p| p.id).collect();
        } else {
            let slots = available_for_children.saturating_sub(1).max(1);
            for pick in picks.iter().take(slots) {
                visible_children.push(pick.id);
            }
        }

        let mut visible_children_with_names: Vec<(String, usize)> = child_entries
            .iter()
            .filter(|(_, id)| visible_children.contains(id))
            .cloned()
            .collect();
        visible_children_with_names.sort_by(|a, b| a.0.cmp(&b.0));

        let mut used = 1;
        let mut remaining = available_for_children;

        let mut hidden_count = child_entries.len().saturating_sub(visible_children_with_names.len());
        let summary_line = if hidden_count > 0 { 1 } else { 0 };

        if visible_children_with_names.len() + summary_line > remaining {
            // If we are still too tight, keep only the most informative child.
            visible_children_with_names.truncate(remaining.saturating_sub(summary_line));
            hidden_count = child_entries.len().saturating_sub(visible_children_with_names.len());
        }

        let mut base_consumption = visible_children_with_names.len() + summary_line;
        if base_consumption > remaining {
            base_consumption = remaining;
        }
        remaining = remaining.saturating_sub(base_consumption);

        let total_weight: f64 = visible_children_with_names
            .iter()
            .filter(|(_, id)| matches!(self.tree.get(*id).file_type, FileType::Directory))
            .map(|(_, id)| self.child_score(*id) + 1.0)
            .sum();

        let mut extras: Vec<usize> = vec![0; visible_children_with_names.len()];
        let mut remaining_extra = remaining;

        if total_weight > 0.0 && remaining_extra > 0 {
            for (idx, (_, id)) in visible_children_with_names
                .iter()
                .enumerate()
                .filter(|(_, (_, id))| matches!(self.tree.get(*id).file_type, FileType::Directory))
            {
                let weight = self.child_score(*id) + 1.0;
                let share = ((weight / total_weight) * remaining as f64).floor() as usize;
                let share = share.min(remaining_extra);
                extras[idx] = share;
                remaining_extra = remaining_extra.saturating_sub(share);
            }

            if remaining_extra > 0 {
                let mut weighted_indices: Vec<(usize, f64)> = visible_children_with_names
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, id))| matches!(self.tree.get(*id).file_type, FileType::Directory))
                    .map(|(idx, (_, id))| (idx, self.child_score(*id) + 1.0))
                    .collect();
                weighted_indices.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

                let mut cycle = weighted_indices.iter().cycle();
                while remaining_extra > 0 {
                    if let Some((idx, _)) = cycle.next() {
                        extras[*idx] += 1;
                        remaining_extra -= 1;
                    }
                }
            }
        }

        for (index, (_, id)) in visible_children_with_names.iter().enumerate() {
            let is_last = if hidden_count > 0 {
                false
            } else {
                index == visible_children_with_names.len() - 1
            };
            let mut next_prefix = prefix_stack.clone();
            next_prefix.push(if is_last {
                PrefixSegment::ShapeL
            } else {
                PrefixSegment::ShapeT
            });

            let allocated = 1 + extras.get(index).copied().unwrap_or(0);
            used += self.render_node(
                *id,
                &mut next_prefix,
                allocated,
                false,
            );
        }

        if hidden_count > 0 && base_consumption > 0 {
            let mut prefix = prefix_stack.clone();
            prefix.push(PrefixSegment::ShapeL);
            let hidden_dirs: usize = child_entries
                .iter()
                .filter(|(_, id)| !visible_children.contains(id))
                .filter(|(_, id)| matches!(self.tree.get(*id).file_type, FileType::Directory))
                .count();
            let hidden_files = hidden_count - hidden_dirs;
            let summary = if hidden_dirs > 0 {
                format!("… ({} dirs, {} files hidden)", hidden_dirs, hidden_files)
            } else {
                format!("… ({} files hidden)", hidden_files)
            };
            let path = self.tree.get(node_for_children).path.clone();
            self.push_entry(summary, path, &prefix, None, true);
            used += 1;
        }

        used
    }
}

#[cfg(test)]
mod test {
    use super::FormattedEntry;
    use crate::file_tree::FileType;
    use std::path;

    #[test]
    fn formatting_works() {
        let formatted = super::format_paths(
            ".",
            vec![
                ("a".to_string(), FileType::File),
                (format!("b{}c", path::MAIN_SEPARATOR), FileType::File),
            ],
            false,
            None,
        );

        let bc_path = format!("b{}c", path::MAIN_SEPARATOR);
        let b_path = format!(".{}b", path::MAIN_SEPARATOR);
        let variant0 = vec![
            FormattedEntry {
                name: ".".to_string(),
                path: ".".to_string(),
                prefix: String::new(),
                link: None,
                virtual_entry: false,
            },
            FormattedEntry {
                name: "a".to_string(),
                path: "a".to_string(),
                prefix: "├── ".to_string(),
                link: None,
                virtual_entry: false,
            },
            FormattedEntry {
                name: "b".to_string(),
                path: b_path.clone(),
                prefix: "└── ".to_string(),
                link: None,
                virtual_entry: false,
            },
            FormattedEntry {
                name: "c".to_string(),
                path: bc_path.clone(),
                prefix: "    └── ".to_string(),
                link: None,
                virtual_entry: false,
            },
        ];

        let variant1 = vec![
            FormattedEntry {
                name: ".".to_string(),
                path: ".".to_string(),
                prefix: String::new(),
                link: None,
                virtual_entry: false,
            },
            FormattedEntry {
                name: "b".to_string(),
                path: b_path.clone(),
                prefix: "├── ".to_string(),
                link: None,
                virtual_entry: false,
            },
            FormattedEntry {
                name: "c".to_string(),
                path: bc_path.clone(),
                prefix: "│   └── ".to_string(),
                link: None,
                virtual_entry: false,
            },
            FormattedEntry {
                name: "a".to_string(),
                path: "a".to_string(),
                prefix: "└── ".to_string(),
                link: None,
                virtual_entry: false,
            },
        ];

        assert!(formatted == variant0 || formatted == variant1);
    }

    #[test]
    fn smart_lines_hints_at_hidden_children() {
        let formatted = super::format_paths(
            ".",
            vec![
                ("a/file1".to_string(), FileType::File),
                ("a/file2".to_string(), FileType::File),
                ("b/file1".to_string(), FileType::File),
                ("c".to_string(), FileType::File),
            ],
            false,
            Some(4),
        );

        let rendered: Vec<(&str, &str)> = formatted
            .iter()
            .map(|e| (e.prefix.as_str(), e.name.as_str()))
            .collect();

        assert_eq!(rendered.len(), 4);
        assert!(rendered.contains(&("", ".")));
        assert!(rendered.contains(&("├── ", "a … (2 hidden)")));
        assert!(rendered.contains(&("├── ", "b … (1 hidden)")));
        assert!(rendered.contains(&("└── ", "c")));
    }

    #[test]
    fn smart_lines_summary_entry_is_virtual() {
        let formatted = super::format_paths(
            ".",
            vec![
                ("alpha/one".to_string(), FileType::File),
                ("beta/one".to_string(), FileType::File),
                ("gamma/one".to_string(), FileType::File),
                ("delta/one".to_string(), FileType::File),
                ("epsilon/one".to_string(), FileType::File),
                ("zeta/one".to_string(), FileType::File),
            ],
            false,
            Some(5),
        );

        let summary = formatted
            .iter()
            .find(|e| e.virtual_entry && e.name.starts_with("… "))
            .expect("summary line present");

        assert_eq!(summary.prefix, "└── ");
        assert!(summary.name.contains("hidden"));
    }
}
