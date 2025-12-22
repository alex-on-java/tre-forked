use super::file_tree::{File, FileTree, FileType, TypeSpecficData};
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
struct DirectorySummary {
    total_descendants: usize,
    child_dirs: usize,
    child_files: usize,
}

#[derive(Debug, Clone)]
struct PlanNode {
    id: usize,
    name: String,
    path: String,
    link: Option<String>,
    is_dir: bool,
    expanded: bool,
    depth: usize,
    summary: Option<DirectorySummary>,
    children: Vec<PlanNode>,
}

fn ensure_dir_suffix(name: &str) -> String {
    if name.ends_with('/') {
        name.to_string()
    } else {
        format!("{}/", name)
    }
}

fn make_prefix(prefix_stack: &[bool], is_last: bool, is_root: bool) -> String {
    if is_root {
        return String::new();
    }

    let mut segments = Vec::new();
    for has_more_siblings in prefix_stack.iter().copied() {
        if has_more_siblings {
            segments.push(PrefixSegment::ShapeI);
        } else {
            segments.push(PrefixSegment::Empty);
        }
    }

    if is_last {
        segments.push(PrefixSegment::ShapeL);
    } else {
        segments.push(PrefixSegment::ShapeT);
    }

    segments.iter().fold(String::new(), |s, seg| {
        s + match seg {
            PrefixSegment::ShapeL => "└── ",
            PrefixSegment::ShapeT => "├── ",
            PrefixSegment::ShapeI => "│   ",
            PrefixSegment::Empty => "    ",
        }
    })
}

fn flatten_skinny_path(tree: &FileTree, start_id: usize) -> (usize, Vec<String>) {
    let mut labels = Vec::new();
    let mut cursor_id = start_id;
    loop {
        let cursor = tree.get(cursor_id);
        labels.push(cursor.display_name.clone());
        if !cursor.is_skinny() {
            break;
        }

        if let Some(children) = cursor.children() {
            if let Some(next_id) = children.values().next() {
                cursor_id = *next_id;
                continue;
            }
        }
        break;
    }

    (cursor_id, labels)
}

fn summarize_directory(file: &File) -> DirectorySummary {
    DirectorySummary {
        total_descendants: file.total_descendants,
        child_dirs: file.child_dir_count,
        child_files: file.child_file_count,
    }
}

fn build_plan(
    tree: &FileTree,
    id: usize,
    depth: usize,
    name_override: Option<String>,
    summary_source: usize,
    collapse_skinny: bool,
) -> PlanNode {
    let file = tree.get(id);
    let link = file.link();
    let is_dir = matches!(file.file_type, FileType::Directory);
    let summary = if is_dir {
        Some(summarize_directory(tree.get(summary_source)))
    } else {
        None
    };
    let mut children = Vec::new();

    if let TypeSpecficData::Directory(child_map) = &file.data {
        for child_id in child_map.values() {
            let child = tree.get(*child_id);
            match child.file_type {
                FileType::Directory => {
                    if collapse_skinny {
                        let (terminal, labels) = flatten_skinny_path(tree, child.id);
                        let display_name = ensure_dir_suffix(&labels.join("/"));
                        children.push(build_plan(
                            tree,
                            terminal,
                            depth + 1,
                            Some(display_name),
                            child.id,
                            collapse_skinny,
                        ));
                    } else {
                        children.push(build_plan(
                            tree,
                            child.id,
                            depth + 1,
                            None,
                            child.id,
                            collapse_skinny,
                        ));
                    }
                }
                _ => {
                    children.push(build_plan(
                        tree,
                        child.id,
                        depth + 1,
                        None,
                        child.id,
                        collapse_skinny,
                    ));
                }
            }
        }
    }

    PlanNode {
        id,
        name: name_override.unwrap_or_else(|| file.display_name.clone()),
        path: file.path.clone(),
        link,
        is_dir,
        expanded: false,
        depth,
        summary,
        children,
    }
}

fn count_lines(plan: &PlanNode) -> usize {
    if plan.expanded {
        1 + plan.children.iter().map(count_lines).sum::<usize>()
    } else {
        1
    }
}

fn collect_expandable<'a>(plan: &'a PlanNode, acc: &mut Vec<&'a PlanNode>) {
    if !plan.is_dir || !plan.expanded {
        return;
    }

    for child in &plan.children {
        if child.is_dir {
            if child.expanded {
                collect_expandable(child, acc);
            } else {
                acc.push(child);
            }
        }
    }
}

fn expand_by_id(plan: &mut PlanNode, target: usize) -> bool {
    if plan.id == target {
        plan.expanded = true;
        return true;
    }

    if !plan.expanded {
        return false;
    }

    for child in &mut plan.children {
        if expand_by_id(child, target) {
            return true;
        }
    }

    false
}

fn expansion_cost(node: &PlanNode) -> usize {
    node.children.iter().map(count_lines).sum::<usize>()
}

fn desirability_score(node: &PlanNode, tree: &FileTree) -> f64 {
    let summary = node
        .summary
        .as_ref()
        .cloned()
        .unwrap_or_else(|| summarize_directory(tree.get(node.id)));
    let breadth = (summary.child_dirs + summary.child_files).max(1) as f64;
    let sparsity = 1.0 / breadth;
    let skinny_bonus = if summary.child_dirs == 1 && summary.child_files == 0 {
        2.0
    } else {
        1.0
    };
    let depth_bonus = 1.0 + (node.depth as f64).sqrt() / 2.0;
    let descendant_penalty = (summary.total_descendants as f64).ln_1p();
    let sibling_variety = node.children.iter().filter(|c| c.is_dir).count().max(1) as f64;

    ((sparsity * 3.0) + skinny_bonus + depth_bonus + sibling_variety.ln_1p())
        / (1.0 + descendant_penalty.sqrt())
}

fn apply_line_budget(plan: &mut PlanNode, max_lines: usize, tree: &FileTree) {
    plan.expanded = true; // always show root children
    let mut used_lines = count_lines(plan);
    if used_lines >= max_lines {
        return;
    }

    let tolerance = (max_lines / 8).max(2);

    loop {
        let mut candidates = Vec::new();
        collect_expandable(plan, &mut candidates);
        if candidates.is_empty() {
            break;
        }

        let mut best: Option<(&PlanNode, f64, usize)> = None;
        for cand in candidates {
            let cost = expansion_cost(cand);
            if cost == 0 {
                continue;
            }
            let score = desirability_score(cand, tree) / cost as f64;
            if let Some((_, best_score, _)) = best {
                if score > best_score {
                    best = Some((cand, score, cost));
                }
            } else {
                best = Some((cand, score, cost));
            }
        }

        let Some((chosen, _, cost)) = best else {
            break;
        };

        if used_lines + cost > max_lines + tolerance {
            break;
        }

        if expand_by_id(plan, chosen.id) {
            used_lines += cost;
        } else {
            break;
        }

        if used_lines >= max_lines {
            break;
        }
    }
}

fn render_name(node: &PlanNode) -> String {
    if node.is_dir {
        let base = ensure_dir_suffix(&node.name);
        if node.expanded {
            base
        } else if let Some(summary) = &node.summary {
            format!(
                "{} ... ({} dirs, {} files, {} total)",
                base, summary.child_dirs, summary.child_files, summary.total_descendants
            )
        } else {
            base
        }
    } else {
        node.name.clone()
    }
}

fn render_plan(
    node: &PlanNode,
    prefix_stack: &mut Vec<bool>,
    result: &mut Vec<FormattedEntry>,
    is_last: bool,
    make_absolute: bool,
) {
    let prefix = make_prefix(prefix_stack, is_last, node.depth == 0);
    let path = if make_absolute {
        fs::canonicalize(&node.path).unwrap().display().to_string()
    } else {
        node.path.clone()
    };

    result.push(FormattedEntry {
        name: render_name(node),
        path,
        prefix,
        link: node.link.clone(),
    });

    if node.expanded {
        let len = node.children.len();
        if node.depth > 0 {
            prefix_stack.push(!is_last);
        }
        for (idx, child) in node.children.iter().enumerate() {
            render_plan(child, prefix_stack, result, idx + 1 == len, make_absolute);
        }
        if node.depth > 0 {
            prefix_stack.pop();
        }
    }
}

pub fn format_paths(
    root_path: &str,
    children: Vec<(String, FileType)>,
    make_absolute: bool,
    max_lines: Option<usize>,
) -> Vec<FormattedEntry> {
    let mut result = Vec::new();
    match FileTree::new(root_path, children) {
        Some(mut tree) => {
            tree.compute_metadata();
            let collapse_skinny = max_lines.is_some();
            let mut plan = build_plan(&tree, tree.root_id, 0, None, tree.root_id, collapse_skinny);
            let line_budget = max_lines.unwrap_or(usize::MAX);
            if line_budget != usize::MAX {
                apply_line_budget(&mut plan, line_budget, &tree);
            } else {
                plan.expanded = true;
                // expand everything to mimic existing behavior
                fn force_expand(node: &mut PlanNode) {
                    if node.is_dir {
                        node.expanded = true;
                        for child in &mut node.children {
                            force_expand(child);
                        }
                    }
                }
                force_expand(&mut plan);
            }

            render_plan(&plan, &mut Vec::new(), &mut result, true, make_absolute);
            result
        }
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
            None,
        );

        let bc_path = format!("b{}c", path::MAIN_SEPARATOR);
        let b_path = format!(".{}b", path::MAIN_SEPARATOR);
        let variant0 = vec![
            FormattedEntry {
                name: "./".to_string(),
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
                name: "b/".to_string(),
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
                name: "./".to_string(),
                path: ".".to_string(),
                prefix: String::new(),
                link: None,
            },
            FormattedEntry {
                name: "b/".to_string(),
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
    fn skinny_paths_are_collapsed_under_budget() {
        let formatted = super::format_paths(
            ".",
            vec![(
                "src/main/java/com/example/App.java".to_string(),
                FileType::File,
            )],
            false,
            Some(6),
        );

        let joined_names: Vec<String> = formatted.iter().map(|f| f.name.clone()).collect();
        assert!(
            joined_names
                .iter()
                .any(|name| name.contains("src/main/java/com/example/")),
            "expected collapsed skinny path"
        );
        assert!(joined_names.iter().any(|name| name.contains("App.java")));
    }

    #[test]
    fn dense_directories_are_summarized() {
        let formatted = super::format_paths(
            "project",
            vec![
                (
                    "project/src/components/Button.tsx".to_string(),
                    FileType::File,
                ),
                (
                    "project/src/components/Card.tsx".to_string(),
                    FileType::File,
                ),
                (
                    "project/src/components/Chip.tsx".to_string(),
                    FileType::File,
                ),
                (
                    "project/src/components/Toast.tsx".to_string(),
                    FileType::File,
                ),
                (
                    "project/src/components/Tooltip.tsx".to_string(),
                    FileType::File,
                ),
                ("project/src/utils/helpers.ts".to_string(), FileType::File),
                ("project/README.md".to_string(), FileType::File),
            ],
            false,
            Some(8),
        );

        assert!(formatted.len() <= 10);
        let render = formatted.iter().map(|f| f.name.clone()).collect::<Vec<_>>();
        assert!(render.iter().any(|n| n.contains("components")));
        assert!(render.iter().any(|n| n.contains("...")));
    }
}
