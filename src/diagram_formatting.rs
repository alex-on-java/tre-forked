use super::file_tree::{File, FileTree, FileType};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone)]
struct DisplayNode {
    name: String,
    path: String,
    link: Option<String>,
    children: Vec<DisplayNode>,
}

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
}

fn make_prefix_from_stack(stack: &[bool]) -> String {
    let mut prefix = String::new();
    if stack.is_empty() {
        return prefix;
    }

    let (rest, last) = stack.split_at(stack.len() - 1);
    for is_last in rest {
        prefix.push_str(if *is_last { "    " } else { "│   " });
    }

    prefix.push_str(if *last.first().unwrap() {
        "└── "
    } else {
        "├── "
    });
    prefix
}

fn emit_display_node(node: &DisplayNode, stack: &mut Vec<bool>, result: &mut Vec<FormattedEntry>) {
    let prefix = make_prefix_from_stack(stack);
    result.push(FormattedEntry {
        name: node.name.clone(),
        path: node.path.clone(),
        prefix,
        link: node.link.clone(),
    });

    let len = node.children.len();
    for (idx, child) in node.children.iter().enumerate() {
        stack.push(idx == len - 1);
        emit_display_node(child, stack, result);
        stack.pop();
    }
}

fn canonicalize_or_fallback(path: &str) -> String {
    fs::canonicalize(path)
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| path.to_string())
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

fn collapse_skinny_path<'a>(tree: &'a FileTree, id: usize) -> (Vec<&'a File>, usize) {
    let mut chain = Vec::new();
    let mut current = tree.get(id);
    chain.push(current);

    while let FileType::Directory = current.file_type {
        if !current.is_skinny() {
            break;
        }
        if let Some(children) = current.children() {
            let next_id = *children.values().next().expect("skinny path child");
            current = tree.get(next_id);
            chain.push(current);
        } else {
            break;
        }
    }

    let terminal_id = chain.last().map(|f| f.id).unwrap_or(id);
    (chain, terminal_id)
}

fn build_skinny_label(chain: &[&File]) -> String {
    if chain.len() <= 1 {
        return chain
            .first()
            .map(|f| f.display_name.clone())
            .unwrap_or_default();
    }

    let names: Vec<String> = chain.iter().map(|f| f.display_name.clone()).collect();
    format!("{}/", names.join("/"))
}

fn descendant_score(file: &File) -> usize {
    // Directories are treated as much more informative than files to favor structure.
    match file.file_type {
        FileType::Directory => file.total_descendants + (file.child_dir_count * 4) + 3,
        FileType::File | FileType::Link => 1,
    }
}

fn planned_line_count(tree: &FileTree, id: usize) -> usize {
    let file = tree.get(id);
    match file.file_type {
        FileType::Directory => {
            let (_, terminal_id) = collapse_skinny_path(tree, id);
            let terminal = tree.get(terminal_id);
            let mut count = 1; // the directory line itself
            if let Some(children) = terminal.children() {
                for child_id in children.values() {
                    count += planned_line_count(tree, *child_id);
                }
            }
            count
        }
        FileType::File | FileType::Link => 1,
    }
}

fn summarize_hidden(descendants: usize) -> DisplayNode {
    DisplayNode {
        name: format!("… ({} entries)", descendants),
        path: String::new(),
        link: None,
        children: vec![],
    }
}

fn render_node(
    tree: &FileTree,
    node_id: usize,
    budget: usize,
    make_absolute: bool,
) -> (DisplayNode, usize, usize) {
    let file = tree.get(node_id);

    let mut used_lines = 1; // this node
    let mut hidden_descendants = 0;

    if let FileType::Directory = file.file_type {
        let (skinny_chain, terminal_id) = collapse_skinny_path(tree, node_id);
        let terminal = tree.get(terminal_id);
        let display_name = build_skinny_label(&skinny_chain);
        let mut node = DisplayNode {
            name: display_name,
            path: if make_absolute {
                canonicalize_or_fallback(&terminal.path)
            } else {
                terminal.path.clone()
            },
            link: terminal.link(),
            children: vec![],
        };

        if budget == 1 {
            hidden_descendants = terminal.total_descendants;
            if hidden_descendants > 0 {
                node.name = format!("{} … ({} entries)", node.name, hidden_descendants);
            }
            return (node, used_lines, hidden_descendants);
        }

        let mut remaining = budget - 1;
        if let Some(children) = terminal.children() {
            let mut scored: Vec<_> = children
                .iter()
                .map(|(name, id)| (name.clone(), *id, descendant_score(tree.get(*id))))
                .collect();
            scored.sort_by(|a, b| {
                b.2.cmp(&a.2)
                    .then_with(|| a.0.cmp(&b.0))
                    .then(Ordering::Equal)
            });

            for (_, child_id, _) in scored {
                if remaining == 0 {
                    hidden_descendants += 1 + tree.get(child_id).total_descendants;
                    continue;
                }

                let allowed = remaining;
                let best_possible = planned_line_count(tree, child_id);
                let child_budget = allowed.min(best_possible);
                let (child_node, lines, hidden) =
                    render_node(tree, child_id, child_budget, make_absolute);
                remaining = remaining.saturating_sub(lines);
                used_lines += lines;
                hidden_descendants += hidden;
                node.children.push(child_node);

                if remaining == 0 {
                    continue;
                }
            }

            if hidden_descendants > 0 {
                if remaining > 0 {
                    let summary = summarize_hidden(hidden_descendants);
                    used_lines += 1;
                    node.children.push(summary);
                } else {
                    node.name = format!("{} … ({} entries)", node.name, hidden_descendants);
                }
            }
        }

        (node, used_lines, hidden_descendants)
    } else {
        let node = DisplayNode {
            name: file.display_name.clone(),
            path: if make_absolute {
                canonicalize_or_fallback(&file.path)
            } else {
                file.path.clone()
            },
            link: file.link(),
            children: vec![],
        };
        (node, used_lines, hidden_descendants)
    }
}

/// Smart formatting mode honoring a maximum number of lines by selectively expanding
/// directories based on metadata-informed heuristics. This favors structural insight over
/// exhaustive listing and attempts to avoid hiding large sections without an indication.
pub fn format_tree_smart_lines(
    tree: &FileTree,
    make_absolute: bool,
    max_lines: usize,
) -> Vec<FormattedEntry> {
    if max_lines == 0 {
        return Vec::new();
    }

    let budget = max_lines.max(1);
    let (plan, _, _) = render_node(tree, tree.root_id, budget, make_absolute);
    let mut result = Vec::new();
    let mut stack = Vec::new();
    emit_display_node(&plan, &mut stack, &mut result);
    if result.len() > max_lines {
        result.truncate(max_lines);
    }
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
            },
            FormattedEntry {
                name: "a".to_string(),
                path: "a".to_string(),
                prefix: "├── ".to_string(),
                link: None,
            },
            FormattedEntry {
                name: "b".to_string(),
                path: b_path.clone(),
                prefix: "└── ".to_string(),
                link: None,
            },
            FormattedEntry {
                name: "c".to_string(),
                path: bc_path.clone(),
                prefix: "    └── ".to_string(),
                link: None,
            },
        ];

        let variant1 = vec![
            FormattedEntry {
                name: ".".to_string(),
                path: ".".to_string(),
                prefix: String::new(),
                link: None,
            },
            FormattedEntry {
                name: "b".to_string(),
                path: b_path.clone(),
                prefix: "├── ".to_string(),
                link: None,
            },
            FormattedEntry {
                name: "c".to_string(),
                path: bc_path.clone(),
                prefix: "│   └── ".to_string(),
                link: None,
            },
            FormattedEntry {
                name: "a".to_string(),
                path: "a".to_string(),
                prefix: "└── ".to_string(),
                link: None,
            },
        ];

        assert!(formatted == variant0 || formatted == variant1);
    }
}
