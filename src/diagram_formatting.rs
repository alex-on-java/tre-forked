use super::file_tree::{File, FileTree, FileType};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

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
    /// Synthetic entries (like ellipsis summaries) should not create editor aliases.
    pub virtual_entry: bool,
}

#[derive(Clone)]
struct PlannedNode {
    name: String,
    path: String,
    link: Option<String>,
    children: Vec<PlannedNode>,
    virtual_entry: bool,
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

/// Format a pre-built FileTree into a list of FormattedEntry.
/// Use this when you need to operate on the tree before formatting (e.g., for --lines).
pub fn format_tree(tree: &FileTree, make_absolute: bool) -> Vec<FormattedEntry> {
    let mut history = HashMap::new();
    let mut result = Vec::new();
    let root = tree.get_root();
    format_file(tree, root, &mut history, &mut result, make_absolute);
    result
}

#[derive(Clone)]
struct ChildDescriptor {
    display_name: String,
    logical_id: usize,
    meta_id: usize,
    is_dir: bool,
    skinny_depth: usize,
}

struct SmartFormatter<'a> {
    tree: &'a FileTree,
    make_absolute: bool,
}

impl<'a> SmartFormatter<'a> {
    fn make_path(&self, path: &str) -> String {
        if self.make_absolute {
            fs::canonicalize(path)
                .unwrap_or_else(|_| PathBuf::from(path))
                .display()
                .to_string()
        } else {
            path.to_string()
        }
    }

    fn collapse_skinny_chain(&self, start_id: usize) -> (usize, usize, String) {
        let mut names = vec![self.tree.get(start_id).display_name.clone()];
        let mut current = start_id;
        let mut depth = 1;

        while self.tree.get(current).is_skinny() {
            if let Some(children) = self.tree.get(current).children() {
                if let Some(next_id) = children.values().next() {
                    current = *next_id;
                    names.push(self.tree.get(current).display_name.clone());
                    depth += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        let label = format!("{}/", names.join("/"));
        (current, depth, label)
    }

    fn collect_children(&self, node_id: usize) -> Vec<ChildDescriptor> {
        let mut descriptors = Vec::new();
        if let Some(children) = self.tree.get(node_id).children() {
            for child_id in children.values() {
                let child = self.tree.get(*child_id);
                let mut skinny_depth = 1;
                let mut display_name = child.display_name.clone();
                let mut logical_id = *child_id;

                if matches!(child.file_type, FileType::Directory) {
                    let (terminal, depth, label) = self.collapse_skinny_chain(*child_id);
                    skinny_depth = depth;
                    logical_id = terminal;
                    display_name = label;
                }

                descriptors.push(ChildDescriptor {
                    display_name: if matches!(child.file_type, FileType::Directory) {
                        display_name
                    } else {
                        child.display_name.clone()
                    },
                    logical_id,
                    meta_id: *child_id,
                    is_dir: matches!(child.file_type, FileType::Directory),
                    skinny_depth,
                });
            }
        }

        descriptors.sort_by(|a, b| {
            if a.is_dir == b.is_dir {
                a.display_name.cmp(&b.display_name)
            } else if a.is_dir {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        });

        descriptors
    }

    fn summarize_hidden(&self, hidden: &[ChildDescriptor], parent_path: &str) -> PlannedNode {
        let hidden_entries: usize = hidden
            .iter()
            .map(|child| 1 + self.tree.get(child.meta_id).total_descendants)
            .sum();
        PlannedNode {
            name: format!("… ({} entries hidden)", hidden_entries),
            path: self.make_path(parent_path),
            link: None,
            children: Vec::new(),
            virtual_entry: true,
        }
    }

    fn collapsed_leaf(&self, child: &ChildDescriptor) -> PlannedNode {
        let meta = self.tree.get(child.meta_id);
        let suffix = if meta.child_dir_count + meta.child_file_count == 0 {
            String::new()
        } else {
            format!(
                " … ({} dirs, {} files)",
                meta.child_dir_count, meta.child_file_count
            )
        };

        PlannedNode {
            name: format!("{}{}", child.display_name, suffix),
            path: self.make_path(&self.tree.get(child.logical_id).path),
            link: None,
            children: Vec::new(),
            virtual_entry: false,
        }
    }

    fn should_expand(&self, child: &ChildDescriptor, remaining: usize, depth: usize) -> bool {
        if !child.is_dir || remaining <= 1 {
            return false;
        }

        let meta = self.tree.get(child.logical_id);
        let branching = meta.child_dir_count + meta.child_file_count;
        if 1 + meta.total_descendants <= remaining {
            return true;
        }

        if branching <= 3 {
            return true;
        }

        if child.skinny_depth > 1 {
            return true;
        }

        depth <= 1
    }

    fn plan_node(
        &self,
        node_id: usize,
        budget: usize,
        depth: usize,
        label_override: Option<String>,
    ) -> (PlannedNode, usize) {
        let file = self.tree.get(node_id);
        let name = label_override.unwrap_or_else(|| file.display_name.clone());
        let mut used = 1;
        let mut planned_children = Vec::new();

        if budget == 0 {
            return (
                PlannedNode {
                    name,
                    path: self.make_path(&file.path),
                    link: file.link(),
                    children: planned_children,
                    virtual_entry: false,
                },
                used,
            );
        }

        if budget == 1 || !matches!(file.file_type, FileType::Directory) {
            return (
                PlannedNode {
                    name,
                    path: self.make_path(&file.path),
                    link: file.link(),
                    children: planned_children,
                    virtual_entry: false,
                },
                used,
            );
        }

        let mut children = self.collect_children(node_id);
        let remaining = budget - 1;
        let mut summary_node: Option<PlannedNode> = None;

        if children.len() > remaining {
            if remaining > 1 {
                let show_count = remaining - 1;
                let hidden: Vec<ChildDescriptor> = children.split_off(show_count);
                summary_node = Some(self.summarize_hidden(&hidden, &file.path));
            } else {
                children.truncate(1);
            }
        }

        let mut remaining_for_children = remaining;
        if summary_node.is_some() && remaining_for_children > 0 {
            remaining_for_children -= 1;
        }

        for child in children {
            if remaining_for_children == 0 {
                break;
            }

            if child.is_dir && self.should_expand(&child, remaining_for_children, depth) {
                let (planned_child, child_used) = self
                    .plan_node(child.logical_id, remaining_for_children, depth + 1, Some(child.display_name.clone()));
                planned_children.push(planned_child);
                used += child_used;
                remaining_for_children = remaining_for_children.saturating_sub(child_used);
            } else {
                planned_children.push(self.collapsed_leaf(&child));
                used += 1;
                remaining_for_children = remaining_for_children.saturating_sub(1);
            }
        }

        if let Some(summary) = summary_node {
            planned_children.push(summary);
            used += 1;
        }

        (
            PlannedNode {
                name,
                path: self.make_path(&file.path),
                link: file.link(),
                children: planned_children,
                virtual_entry: false,
            },
            used,
        )
    }
}

fn render_planned(
    node: &PlannedNode,
    result: &mut Vec<FormattedEntry>,
    ancestor_branches: Vec<bool>,
    connector: Option<bool>,
) {
    let mut prefix = String::new();
    for has_more in &ancestor_branches {
        if *has_more {
            prefix.push_str("│   ");
        } else {
            prefix.push_str("    ");
        }
    }

    if let Some(has_more) = connector {
        if has_more {
            prefix.push_str("├── ");
        } else {
            prefix.push_str("└── ");
        }
    }

    result.push(FormattedEntry {
        name: node.name.clone(),
        path: node.path.clone(),
        prefix,
        link: node.link.clone(),
        virtual_entry: node.virtual_entry,
    });

    let children_len = node.children.len();
    for (idx, child) in node.children.iter().enumerate() {
        let is_last = idx + 1 == children_len;
        let mut next_branches = ancestor_branches.clone();
        if connector.is_some() {
            next_branches.push(!is_last);
        }
        render_planned(child, result, next_branches, Some(!is_last));
    }
}

pub fn format_tree_with_line_budget(
    tree: &FileTree,
    make_absolute: bool,
    max_lines: usize,
) -> Vec<FormattedEntry> {
    if max_lines == 0 {
        return Vec::new();
    }

    let full = format_tree(tree, make_absolute);
    if full.len() <= max_lines {
        return full;
    }

    let formatter = SmartFormatter {
        tree,
        make_absolute,
    };

    let (plan, _) = formatter.plan_node(tree.root_id, max_lines, 0, None);
    let mut result = Vec::new();
    render_planned(&plan, &mut result, Vec::new(), None);
    result
}

/// Convenience function that builds a tree and formats it in one step.
/// Primarily used for backwards compatibility and tests.
#[allow(dead_code)]
pub fn format_paths(
    root_path: &str,
    children: Vec<(String, FileType)>,
    make_absolute: bool,
) -> Vec<FormattedEntry> {
    match FileTree::new(root_path, children) {
        Some(tree) => format_tree(&tree, make_absolute),
        None => Vec::new(),
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
    fn smart_formatting_respects_budget_and_collapses() {
        let mut tree = crate::file_tree::FileTree::new(
            ".",
            vec![
                ("src/main/java/de/konux/healthservice/App.java".to_string(), FileType::File),
                ("src/lib.rs".to_string(), FileType::File),
                ("README.md".to_string(), FileType::File),
            ],
        )
        .unwrap();
        tree.compute_metadata();

        let formatted = super::format_tree_with_line_budget(&tree, false, 5);

        assert!(formatted.len() <= 5);
        assert_eq!(formatted[0].virtual_entry, false);
        // Expect skinny path collapse to appear as a single entry containing the chain
        assert!(formatted
            .iter()
            .any(|entry| entry.name.contains("main/java/de/konux/healthservice")));
    }
}
