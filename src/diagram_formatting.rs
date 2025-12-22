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
}

#[derive(Debug, Clone)]
struct PlannedEntry {
    display_name: String,
    children: Vec<PlannedEntry>,
    path: String,
    link: Option<String>,
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

/// Render the tree using a "smart" line budget that prioritizes structural signal.
///
/// This allocator purposely trades strict accuracy for readability: it will always
/// surface top-level structure, expand skinny paths aggressively, preview dense
/// directories, and append explicit summaries when content is hidden.
pub fn format_tree_smart(tree: &FileTree, max_lines: usize, make_absolute: bool) -> Vec<FormattedEntry> {
    let mut planner = SmartPlanner::new(tree, max_lines);
    let plan = planner.build_plan();
    render_plan(&plan, make_absolute)
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

fn render_plan(root: &PlannedEntry, make_absolute: bool) -> Vec<FormattedEntry> {
    let mut rendered = Vec::new();
    render_node(root, &mut Vec::new(), &mut rendered, make_absolute);
    rendered
}

fn render_node(
    node: &PlannedEntry,
    spine: &mut Vec<bool>,
    rendered: &mut Vec<FormattedEntry>,
    make_absolute: bool,
) {
    let prefix = spine
        .iter()
        .enumerate()
        .fold(String::new(), |mut acc, (idx, is_last)| {
            let is_tail = idx == spine.len() - 1;
            acc.push_str(match (is_tail, is_last) {
                (true, true) => "└── ",
                (true, false) => "├── ",
                (false, true) => "    ",
                (false, false) => "│   ",
            });
            acc
        });

    let path = if make_absolute {
        fs::canonicalize(&node.path)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| node.path.clone())
    } else {
        node.path.clone()
    };

    rendered.push(FormattedEntry {
        name: node.display_name.clone(),
        path,
        prefix,
        link: node.link.clone(),
    });

    let child_len = node.children.len();
    for (idx, child) in node.children.iter().enumerate() {
        spine.push(idx + 1 == child_len);
        render_node(child, spine, rendered, make_absolute);
        spine.pop();
    }
}

struct SmartPlanner<'a> {
    tree: &'a FileTree,
    budget: usize,
}

impl<'a> SmartPlanner<'a> {
    fn new(tree: &'a FileTree, budget: usize) -> Self {
        Self { tree, budget }
    }

    fn build_plan(&mut self) -> PlannedEntry {
        // Ensure at least the root is shown.
        let mut plan = self.plan_node(self.tree.root_id, self.budget, 0);

        // If the repository is tiny, fall back to full formatting for stability.
        if plan.count_lines() <= self.budget {
            return plan;
        }

        // Always surface top-level structure even when we ran out of budget.
        if plan.children.is_empty() {
            let root = self.tree.get(self.tree.root_id);
            if let Some(children) = root.children() {
                plan.children = children
                    .values()
                    .map(|id| self.shallow_placeholder(*id))
                    .collect();
            }
        }

        plan
    }

    fn shallow_placeholder(&self, node_id: usize) -> PlannedEntry {
        let node = self.tree.get(node_id);
        PlannedEntry {
            display_name: node.display_name.clone(),
            children: vec![self.summary_child(node)],
            path: node.path.clone(),
            link: node.link(),
        }
    }

    fn plan_node(&mut self, node_id: usize, budget: usize, depth: usize) -> PlannedEntry {
        let node = self.tree.get(node_id);
        // Collapse skinny paths into a single informative segment.
        let (leaf_id, collapsed_name) = self.collapse_skinny_path(node_id);
        let leaf = self.tree.get(leaf_id);
        let display_name = collapsed_name.unwrap_or_else(|| node.display_name.clone());

        let mut planned_children = Vec::new();
        let remaining_budget = budget.saturating_sub(1);

        if let FileType::Directory = leaf.file_type {
            if remaining_budget > 0 {
                let (children, _used) = self.plan_children(leaf_id, remaining_budget, depth + 1);
                planned_children = children;
            }
        }

        PlannedEntry {
            display_name,
            children: planned_children,
            path: leaf.path.clone(),
            link: leaf.link(),
        }
    }

    fn plan_children(
        &mut self,
        parent_id: usize,
        budget: usize,
        depth: usize,
    ) -> (Vec<PlannedEntry>, usize) {
        let parent = self.tree.get(parent_id);
        let mut lines_used = 0usize;
        let mut planned_children = Vec::new();
        let mut available = budget;

        let mut children: Vec<usize> = parent
            .children()
            .map(|c| c.values().cloned().collect())
            .unwrap_or_default();

        if children.is_empty() {
            return (planned_children, lines_used);
        }

        // Heuristic ordering: directories first, then by descendant count (larger first).
        children.sort_by(|a, b| {
            let file_a = self.tree.get(*a);
            let file_b = self.tree.get(*b);
            let dir_bias = match (&file_a.file_type, &file_b.file_type) {
                (FileType::Directory, FileType::File | FileType::Link) => std::cmp::Ordering::Less,
                (FileType::File | FileType::Link, FileType::Directory) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            };
            if dir_bias != std::cmp::Ordering::Equal {
                return dir_bias;
            }
            file_b
                .total_descendants
                .cmp(&file_a.total_descendants)
                .then_with(|| file_a.display_name.cmp(&file_b.display_name))
        });

        let child_count = children.len();
        let should_expand_all = self.should_expand_all(parent, available, depth, child_count);

        if should_expand_all {
            for child_id in children {
                if available == 0 {
                    break;
                }
                let planned = self.plan_node(child_id, available, depth + 1);
                available = available.saturating_sub(planned.count_lines());
                lines_used += planned.count_lines();
                planned_children.push(planned);
            }
            return (planned_children, lines_used);
        }

        // Dense directories: preview the most interesting children, then summarize the rest.
        let preview_cap = 3usize.max(available.min(3));
        let mut consumed_children = 0usize;
        for child_id in children.iter().copied() {
            if planned_children.len() >= preview_cap || available == 0 {
                break;
            }

            let planned = self.plan_node(child_id, available, depth + 1);
            let cost = planned.count_lines();
            if cost > available && !planned_children.is_empty() {
                break;
            }

            available = available.saturating_sub(cost);
            lines_used += cost;
            consumed_children += 1;
            planned_children.push(planned);
        }

        let hidden_children = child_count.saturating_sub(consumed_children);
        if hidden_children > 0 {
            let remaining_dirs = children
                .iter()
                .skip(consumed_children)
                .filter(|id| matches!(self.tree.get(**id).file_type, FileType::Directory))
                .count();
            let remaining_files = hidden_children - remaining_dirs;

            if available > 0 || planned_children.is_empty() {
                planned_children.push(PlannedEntry {
                    display_name: format!(
                        "… {} more ({} dirs, {} files)",
                        hidden_children, remaining_dirs, remaining_files
                    ),
                    children: Vec::new(),
                    path: self.tree.get(parent_id).path.clone(),
                    link: None,
                });
                lines_used += 1;
            }
        }

        (planned_children, lines_used)
    }

    fn should_expand_all(&self, node: &File, budget: usize, depth: usize, child_count: usize) -> bool {
        if budget == 0 {
            return false;
        }

        if child_count <= 3 {
            return true;
        }

        // If the number of children dwarfs the remaining budget, prefer summarizing.
        if child_count > budget.saturating_mul(2) {
            return false;
        }

        // If the entire subtree fits in budget, just expand it.
        if node.total_descendants + 1 <= budget {
            return true;
        }

        // Shallow nodes are important for understanding structure.
        if depth <= 1 && child_count <= 12 {
            return true;
        }

        // Prefer expanding sparse directories over dense ones.
        let density = node.total_descendants as f64 / child_count as f64;
        density < 2.5
    }

    fn collapse_skinny_path(&self, start_id: usize) -> (usize, Option<String>) {
        let mut current = start_id;
        let mut segments = Vec::new();
        let mut traversed = 0usize;

        while self.tree.get(current).is_skinny() {
            let file = self.tree.get(current);
            segments.push(file.display_name.clone());
            if let Some(child_id) = file
                .children()
                .and_then(|c| c.values().next())
                .cloned()
            {
                current = child_id;
                traversed += 1;
            } else {
                break;
            }
        }

        if traversed == 0 {
            return (current, None);
        }

        // Include the final leaf in the collapsed path.
        segments.push(self.tree.get(current).display_name.clone());
        let collapsed = segments.join("/") + "/";
        (current, Some(collapsed))
    }

    fn summary_child(&self, node: &File) -> PlannedEntry {
        let (dirs, files) = (
            node.child_dir_count,
            node.child_file_count,
        );
        PlannedEntry {
            display_name: format!("… ({} dirs, {} files)", dirs, files),
            children: Vec::new(),
            path: node.path.clone(),
            link: None,
        }
    }
}

impl PlannedEntry {
    fn count_lines(&self) -> usize {
        1 + self.children.iter().map(Self::count_lines).sum::<usize>()
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
    fn smart_formats_skinny_paths_and_summaries() {
        let mut tree = crate::file_tree::FileTree::new(
            ".",
            vec![
                (
                    "src/main/java/de/konux/healthservice/core/App.java".to_string(),
                    FileType::File,
                ),
                (
                    "src/main/java/de/konux/healthservice/infrastructure/Infra.java".to_string(),
                    FileType::File,
                ),
                (
                    "src/main/java/de/konux/healthservice/presentation/rest/Rest.java".to_string(),
                    FileType::File,
                ),
                ("README.md".to_string(), FileType::File),
                ("node_modules/lodash/index.js".to_string(), FileType::File),
            ],
        )
        .unwrap();

        tree.compute_metadata();

        let rendered = super::format_tree_smart(&tree, 8, false);
        let names: Vec<_> = rendered.iter().map(|e| e.name.as_str()).collect();

        assert!(names.iter().any(|name| name.contains("src/main/java/de/konux/healthservice/")));
        assert!(rendered.len() <= 8);
    }

    #[test]
    fn smart_summarizes_dense_directories() {
        let mut tree = crate::file_tree::FileTree::new(
            ".",
            vec![
                ("docs/readme.md".to_string(), FileType::File),
                ("pkg/a.rs".to_string(), FileType::File),
                ("pkg/b.rs".to_string(), FileType::File),
                ("pkg/c.rs".to_string(), FileType::File),
                ("pkg/d.rs".to_string(), FileType::File),
                ("pkg/e.rs".to_string(), FileType::File),
                ("pkg/f.rs".to_string(), FileType::File),
                ("pkg/g.rs".to_string(), FileType::File),
                ("pkg/h.rs".to_string(), FileType::File),
                ("pkg/i.rs".to_string(), FileType::File),
                ("pkg/j.rs".to_string(), FileType::File),
                ("pkg/k.rs".to_string(), FileType::File),
                ("pkg/l.rs".to_string(), FileType::File),
                ("pkg/m.rs".to_string(), FileType::File),
                ("pkg/n.rs".to_string(), FileType::File),
                ("pkg/o.rs".to_string(), FileType::File),
                ("pkg/p.rs".to_string(), FileType::File),
                ("pkg/q.rs".to_string(), FileType::File),
                ("pkg/r.rs".to_string(), FileType::File),
                ("pkg/s.rs".to_string(), FileType::File),
                ("pkg/t.rs".to_string(), FileType::File),
            ],
        )
        .unwrap();

        tree.compute_metadata();

        let rendered = super::format_tree_smart(&tree, 6, false);
        let summary = rendered
            .iter()
            .find(|entry| entry.name.contains("more"))
            .map(|e| e.name.clone())
            .unwrap_or_default();

        assert!(summary.contains("more"));
    }
}
