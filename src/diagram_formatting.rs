use super::file_tree::{File, FileTree, FileType};
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
struct ExpansionCandidate {
    id: usize,
    score: f64,
    cost: usize,
    depth: usize,
}

impl Eq for ExpansionCandidate {}

impl PartialEq for ExpansionCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score
            && self.cost == other.cost
            && self.depth == other.depth
            && self.id == other.id
    }
}

impl PartialOrd for ExpansionCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ExpansionCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Highest score first, then shallower depth, then smaller cost (cheap wins ties)
        other
            .score
            .partial_cmp(&self.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.depth.cmp(&other.depth))
            .then_with(|| other.cost.cmp(&self.cost))
            .then_with(|| self.id.cmp(&other.id))
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

fn summarize_children(file: &File) -> Option<String> {
    if file.child_dir_count == 0 && file.child_file_count == 0 {
        None
    } else if file.child_dir_count == 0 {
        Some(format!("... ({} files)", file.child_file_count))
    } else if file.child_file_count == 0 {
        Some(format!("... ({} dirs)", file.child_dir_count))
    } else {
        Some(format!(
            "... ({} dirs, {} files)",
            file.child_dir_count, file.child_file_count
        ))
    }
}

fn collapse_skinny_path(tree: &FileTree, start_id: usize) -> (String, usize, usize) {
    let mut labels = Vec::new();
    let mut current_id = start_id;
    let mut skipped = 0;

    loop {
        let node = tree.get(current_id);
        labels.push(node.display_name.clone());

        if !matches!(node.file_type, FileType::Directory) || !node.is_skinny() {
            break;
        }

        if let Some(children) = node.children() {
            if let Some((&_, next_id)) = children.first() {
                current_id = *next_id;
                skipped += 1;
                continue;
            }
        }

        break;
    }

    let mut name = labels.join("/");
    if matches!(tree.get(current_id).file_type, FileType::Directory) && !name.ends_with('/') {
        name.push('/');
    }

    (name, current_id, skipped)
}

fn depth_of(tree: &FileTree, file: &File) -> usize {
    let mut depth = 0;
    let mut current = file;
    while let Some(parent) = tree.get_parent(current) {
        depth += 1;
        current = parent;
    }
    depth
}

fn make_candidate(tree: &FileTree, id: usize) -> ExpansionCandidate {
    let file = tree.get(id);
    let fanout = (file.child_dir_count + file.child_file_count).max(1) as f64;
    let deep_bonus = (file.total_descendants as f64 + 1.0).ln_1p();
    let skinny_bonus = if file.is_skinny() { 2.0 } else { 1.0 };
    let dir_weight = (file.child_dir_count as f64 + 1.0) / fanout;
    let score = deep_bonus * skinny_bonus * dir_weight / fanout;
    let cost = (file.child_dir_count + file.child_file_count) as usize;
    ExpansionCandidate {
        id,
        score,
        cost,
        depth: depth_of(tree, file),
    }
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

fn push_entry(
    tree: &FileTree,
    node_id: usize,
    name: String,
    prefix: String,
    make_absolute: bool,
    result: &mut Vec<FormattedEntry>,
) {
    let file = tree.get(node_id);
    let path = if make_absolute {
        fs::canonicalize(&file.path).unwrap().display().to_string()
    } else {
        file.path.clone()
    };

    result.push(FormattedEntry {
        name,
        path,
        prefix,
        link: file.link(),
    });
}

fn format_smart_node(
    tree: &FileTree,
    node_id: usize,
    prefix: String,
    child_indent: String,
    expanded: &HashSet<usize>,
    summaries: &HashMap<usize, String>,
    make_absolute: bool,
    result: &mut Vec<FormattedEntry>,
) {
    let (mut display_name, terminal_id, skipped) = collapse_skinny_path(tree, node_id);
    if skipped > 0 {
        display_name = format!("{} (skinny)", display_name.trim_end_matches('/'));
    }

    if let Some(summary) = summaries.get(&terminal_id) {
        display_name = format!("{} {}", display_name, summary);
    }

    push_entry(tree, terminal_id, display_name, prefix, make_absolute, result);

    if !expanded.contains(&terminal_id) {
        return;
    }

    let node = tree.get(terminal_id);
    if let Some(children) = node.children() {
        let len = children.len();
        for (idx, child_id) in children.values().enumerate() {
            let is_last = idx + 1 == len;
            let connector = if is_last { "└── " } else { "├── " };
            let child_prefix = format!("{}{}", child_indent, connector);
            let next_indent = format!("{}{}", child_indent, if is_last { "    " } else { "│   " });
            format_smart_node(
                tree,
                *child_id,
                child_prefix,
                next_indent,
                expanded,
                summaries,
                make_absolute,
                result,
            );
        }
    }
}

pub fn format_tree_with_budget(
    tree: &FileTree,
    make_absolute: bool,
    max_lines: usize,
) -> Vec<FormattedEntry> {
    if max_lines == 0 {
        return Vec::new();
    }

    let mut expanded: HashSet<usize> = HashSet::new();
    let mut summaries: HashMap<usize, String> = HashMap::new();
    let root_id = tree.root_id;
    expanded.insert(root_id);

    let root_children: Vec<usize> = tree
        .get_root()
        .children()
        .map(|c| c.values().cloned().collect())
        .unwrap_or_default();

    // Root and its direct children are always visible to reveal top-level structure.
    let mut line_count = 1 + root_children.len();
    let mut frontier: BinaryHeap<ExpansionCandidate> = BinaryHeap::new();
    for child in &root_children {
        let (_, terminal, _) = collapse_skinny_path(tree, *child);
        if matches!(tree.get(terminal).file_type, FileType::Directory) {
            frontier.push(make_candidate(tree, terminal));
        }
    }

    while let Some(candidate) = frontier.pop() {
        if expanded.contains(&candidate.id) {
            continue;
        }

        if candidate.cost == 0 {
            continue;
        }

        let projected = line_count + candidate.cost;
        if projected > max_lines && line_count >= max_lines {
            summaries.insert(candidate.id, summarize_children(tree.get(candidate.id)).unwrap());
            continue;
        }

        if projected <= max_lines || candidate.cost <= 2 {
            expanded.insert(candidate.id);
            line_count = projected;

            if let Some(children) = tree.get(candidate.id).children() {
                for child in children.values() {
                    let (_, terminal, _) = collapse_skinny_path(tree, *child);
                    if matches!(tree.get(terminal).file_type, FileType::Directory) {
                        frontier.push(make_candidate(tree, terminal));
                    }
                }
            }
        } else {
            summaries.insert(candidate.id, summarize_children(tree.get(candidate.id)).unwrap());
        }
    }

    // Any directory not expanded should still advertise that it's holding more.
    for (_, file) in tree.storage.iter() {
        if matches!(file.file_type, FileType::Directory)
            && !expanded.contains(&file.id)
            && file.children_count() > 0
        {
            summaries.entry(file.id).or_insert_with(|| {
                summarize_children(file).unwrap_or_else(|| "...".to_string())
            });
        }
    }

    let mut result = Vec::new();
    format_smart_node(
        tree,
        root_id,
        String::new(),
        String::new(),
        &expanded,
        &summaries,
        make_absolute,
        &mut result,
    );
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

    #[test]
    fn smart_formatting_prefers_structure() {
        let mut tree = crate::file_tree::FileTree::new(
            ".",
            vec![
                ("src/main/java/com/App.java".to_string(), FileType::File),
                ("dense/a.rs".to_string(), FileType::File),
                ("dense/b.rs".to_string(), FileType::File),
                ("dense/c.rs".to_string(), FileType::File),
                ("dense/d.rs".to_string(), FileType::File),
                ("README.md".to_string(), FileType::File),
            ],
        )
        .unwrap();
        tree.compute_metadata();

        let formatted = super::format_tree_with_budget(&tree, false, 6);
        let skinny_line = formatted
            .iter()
            .find(|e| e.name.contains("src/main/java/com"))
            .expect("skinny path collapsed");
        assert!(skinny_line.name.contains("skinny"));

        let dense_line = formatted
            .iter()
            .find(|e| e.name.starts_with("dense"))
            .expect("dense directory shown");
        assert!(dense_line.name.contains("... (4 files)"));

        assert!(formatted.iter().any(|e| e.name.ends_with("App.java")));
    }
}
