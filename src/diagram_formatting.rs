use super::file_tree::{File, FileTree, FileType};
use ordered_float::OrderedFloat;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
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
}

#[derive(Debug, Clone)]
struct DisplayNode {
    tree_id: usize,
    name: String,
    path: String,
    link: Option<String>,
    children: Vec<usize>,
    shown_child_tree_ids: Vec<usize>,
    depth: usize,
}

#[derive(Debug, Clone)]
struct Candidate {
    score: OrderedFloat<f64>,
    depth: usize,
    parent_display_id: usize,
    child_tree_id: usize,
    collapsed_label: String,
    collapsed_path: String,
    link: Option<String>,
    covered_tree_ids: Vec<usize>,
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score && self.child_tree_id == other.child_tree_id
    }
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for max-heap semantics
        self.score
            .cmp(&other.score)
            .then_with(|| other.depth.cmp(&self.depth))
            .then_with(|| self.child_tree_id.cmp(&other.child_tree_id))
    }
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

#[allow(dead_code)]
pub fn format_paths(
    root_path: &str,
    children: Vec<(String, FileType)>,
    make_absolute: bool,
) -> Vec<FormattedEntry> {
    format_paths_with_limit(root_path, children, make_absolute, None)
}

pub fn format_paths_with_limit(
    root_path: &str,
    children: Vec<(String, FileType)>,
    make_absolute: bool,
    max_lines: Option<usize>,
) -> Vec<FormattedEntry> {
    let mut history = HashMap::new();
    let mut result = Vec::new();
    match FileTree::new(root_path, children) {
        Some(mut tree) => {
            if max_lines.is_none() {
                let root = tree.get_root();
                format_file(&tree, root, &mut history, &mut result, make_absolute);
                return result;
            }

            tree.compute_metadata();
            let display_tree = build_smart_display_tree(&tree, max_lines.unwrap());
            render_display_tree(&display_tree, make_absolute, &mut result);
            result
        }
        None => Vec::new(),
    }
}

fn build_display_node(
    tree_id: usize,
    name: String,
    path: String,
    link: Option<String>,
    depth: usize,
) -> DisplayNode {
    DisplayNode {
        tree_id,
        name,
        path,
        link,
        children: Vec::new(),
        shown_child_tree_ids: Vec::new(),
        depth,
    }
}

fn collapse_skinny_path(tree: &FileTree, start_id: usize) -> (usize, String, Vec<usize>) {
    let mut cursor = start_id;
    let mut parts = vec![tree.get(cursor).display_name.clone()];
    let mut covered = vec![cursor];

    while let FileType::Directory = tree.get(cursor).file_type {
        if !tree.get(cursor).is_skinny() {
            break;
        }

        if let Some(children) = tree.get(cursor).children() {
            if let Some(&child_id) = children.values().next() {
                cursor = child_id;
                parts.push(tree.get(cursor).display_name.clone());
                covered.push(cursor);
                continue;
            }
        }
        break;
    }

    (cursor, parts.join("/"), covered)
}

fn compute_score(tree: &FileTree, node_id: usize, depth: usize) -> OrderedFloat<f64> {
    let node = tree.get(node_id);
    let structure_bonus = (node.child_dir_count as f64 + 1.0).ln_1p();
    let descendant_bonus = (node.total_descendants as f64 + 1.0).ln_1p();
    let skinny_bonus = if node.is_skinny() { 1.8 } else { 1.0 };
    let density_penalty = 1.0
        / ((node.child_dir_count + node.child_file_count + 1) as f64)
            .sqrt()
            .max(1.0);
    let depth_penalty = 1.0 / ((depth as f64 + 1.0).sqrt());

    OrderedFloat(
        (1.0 + structure_bonus + descendant_bonus) * skinny_bonus * density_penalty * depth_penalty,
    )
}

fn enqueue_children(
    tree: &FileTree,
    display_nodes: &[DisplayNode],
    parent_display_id: usize,
    queue: &mut BinaryHeap<Candidate>,
    visited: &HashSet<usize>,
) {
    let parent_display = &display_nodes[parent_display_id];
    let parent_tree_node = tree.get(parent_display.tree_id);
    if let Some(children) = parent_tree_node.children() {
        for child_id in children.values() {
            if visited.contains(child_id) {
                continue;
            }
            let (collapsed_id, label, covered) = collapse_skinny_path(tree, *child_id);
            let score = compute_score(tree, collapsed_id, parent_display.depth + 1);
            queue.push(Candidate {
                score,
                depth: parent_display.depth + 1,
                parent_display_id,
                child_tree_id: *child_id,
                collapsed_label: label,
                collapsed_path: tree.get(collapsed_id).path.clone(),
                link: tree.get(collapsed_id).link(),
                covered_tree_ids: covered,
            });
        }
    }
}

fn build_smart_display_tree(tree: &FileTree, max_lines: usize) -> Vec<DisplayNode> {
    let mut display_nodes = Vec::new();
    let mut visited: HashSet<usize> = HashSet::new();
    let mut queue: BinaryHeap<Candidate> = BinaryHeap::new();

    let root = tree.get_root();
    display_nodes.push(build_display_node(
        root.id,
        root.display_name.clone(),
        root.path.clone(),
        root.link(),
        0,
    ));
    visited.insert(root.id);

    // Top-level visibility is mandatory regardless of budget.
    if let Some(children) = root.children() {
        for child_id in children.values() {
            let (collapsed_id, label, covered) = collapse_skinny_path(tree, *child_id);
            let child_display_id = display_nodes.len();
            display_nodes.push(build_display_node(
                collapsed_id,
                label,
                tree.get(collapsed_id).path.clone(),
                tree.get(collapsed_id).link(),
                1,
            ));
            display_nodes[0].children.push(child_display_id);
            display_nodes[0].shown_child_tree_ids.push(*child_id);
            for id in covered {
                visited.insert(id);
            }
        }
    }

    let line_budget = max_lines.max(display_nodes.len());
    for child in display_nodes[0].children.clone() {
        enqueue_children(tree, &display_nodes, child, &mut queue, &visited);
    }

    while display_nodes.len() < line_budget {
        let candidate = match queue.pop() {
            Some(c) => c,
            None => break,
        };

        if visited.contains(&candidate.child_tree_id) {
            continue;
        }

        let display_id = display_nodes.len();
        let depth = candidate.depth;
        display_nodes.push(build_display_node(
            tree.get(
                candidate
                    .covered_tree_ids
                    .last()
                    .copied()
                    .unwrap_or(candidate.child_tree_id),
            )
            .id,
            candidate.collapsed_label.clone(),
            candidate.collapsed_path.clone(),
            candidate.link.clone(),
            depth,
        ));

        display_nodes[candidate.parent_display_id]
            .children
            .push(display_id);
        display_nodes[candidate.parent_display_id]
            .shown_child_tree_ids
            .push(candidate.child_tree_id);

        for id in &candidate.covered_tree_ids {
            visited.insert(*id);
        }

        if let FileType::Directory = tree
            .get(
                candidate
                    .covered_tree_ids
                    .last()
                    .copied()
                    .unwrap_or(candidate.child_tree_id),
            )
            .file_type
        {
            enqueue_children(tree, &display_nodes, display_id, &mut queue, &visited);
        }
    }

    sort_children(&mut display_nodes, 0);
    annotate_hidden_children(tree, &mut display_nodes, 0);
    display_nodes
}

fn sort_children(display_nodes: &mut [DisplayNode], node_id: usize) {
    let mut ordered_children = display_nodes[node_id].children.clone();
    ordered_children.sort_by(|a, b| display_nodes[*a].name.cmp(&display_nodes[*b].name));
    display_nodes[node_id].children = ordered_children.clone();

    for child in ordered_children {
        sort_children(display_nodes, child);
    }
}

fn annotate_hidden_children(tree: &FileTree, display_nodes: &mut [DisplayNode], node_id: usize) {
    let total = display_nodes[node_id].children.len();
    let tree_node = tree.get(display_nodes[node_id].tree_id);
    if let FileType::Directory = tree_node.file_type {
        let mut shown_dirs = 0;
        let mut shown_files = 0;
        for child_tree_id in &display_nodes[node_id].shown_child_tree_ids {
            match tree.get(*child_tree_id).file_type {
                FileType::Directory => shown_dirs += 1,
                _ => shown_files += 1,
            }
        }
        let hidden_dirs = tree_node.child_dir_count.saturating_sub(shown_dirs);
        let hidden_files = tree_node.child_file_count.saturating_sub(shown_files);
        if hidden_dirs > 0 || hidden_files > 0 {
            display_nodes[node_id].name = format!(
                "{}    … ({} dirs, {} files)",
                display_nodes[node_id].name, hidden_dirs, hidden_files
            );
        }
    }

    for idx in 0..total {
        let child_id = display_nodes[node_id].children[idx];
        annotate_hidden_children(tree, display_nodes, child_id);
    }
}

fn render_display_tree(
    display_tree: &[DisplayNode],
    make_absolute: bool,
    result: &mut Vec<FormattedEntry>,
) {
    fn render_node(
        display_tree: &[DisplayNode],
        node_id: usize,
        active: &mut Vec<bool>,
        make_absolute: bool,
        result: &mut Vec<FormattedEntry>,
    ) {
        let node = &display_tree[node_id];
        let mut prefix = String::new();
        for is_active in active.iter() {
            prefix.push_str(if *is_active { "│   " } else { "    " });
        }

        if !active.is_empty() {
            if *active.last().unwrap() {
                prefix.push_str("├── ");
            } else {
                prefix.push_str("└── ");
            }
        }

        let path = if make_absolute {
            fs::canonicalize(&node.path)
                .unwrap_or_else(|_| node.path.clone().into())
                .display()
                .to_string()
        } else {
            node.path.clone()
        };

        result.push(FormattedEntry {
            name: node.name.clone(),
            path,
            prefix,
            link: node.link.clone(),
        });

        let child_count = node.children.len();
        for (idx, child_id) in node.children.iter().enumerate() {
            if idx + 1 < child_count {
                active.push(true);
            } else {
                active.push(false);
            }
            render_node(display_tree, *child_id, active, make_absolute, result);
            active.pop();
        }
    }

    render_node(display_tree, 0, &mut Vec::new(), make_absolute, result);
}

#[cfg(test)]
mod test {
    use super::{format_paths_with_limit, FormattedEntry};
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

    #[test]
    fn skinny_paths_collapse_into_single_entry() {
        let formatted = format_paths_with_limit(
            ".",
            vec![(
                "src/main/java/de/konux/App.java".to_string(),
                FileType::File,
            )],
            false,
            Some(5),
        );

        assert!(formatted
            .iter()
            .any(|entry| entry.name.contains("src/main/java/de/konux")));
    }

    #[test]
    fn dense_directories_are_summarized() {
        let formatted = format_paths_with_limit(
            ".",
            vec![
                ("pkg/a.rs".to_string(), FileType::File),
                ("pkg/b.rs".to_string(), FileType::File),
                ("pkg/c.rs".to_string(), FileType::File),
                ("keep.txt".to_string(), FileType::File),
            ],
            false,
            Some(3),
        );

        let pkg_line = formatted
            .iter()
            .find(|entry| entry.name.starts_with("pkg"))
            .expect("pkg should be displayed");

        assert!(pkg_line.name.contains("… (0 dirs, 3 files)"));
    }
}
