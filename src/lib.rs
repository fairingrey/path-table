#![warn(
    nonstandard_style,
    rust_2018_idioms,
    future_incompatible,
    missing_debug_implementations
)]

//! Generic types for path-based routing.

use std::collections::HashMap;

/// A generic path-based routing table, terminating with resources `R`.
//
// The implementation uses a very simple-minded tree structure. `PathTable` is a node,
// with branches corresponding to the next path segment. For concrete segments, the
// `next` table gives the available string matches. For the (at most one) wildcard match,
// the `wildcard` field contains the branch.
//
// If the current path itself is a route, the `accept` field says what resource it contains.
#[derive(Clone)]
pub struct PathTable<R> {
    accept: Option<R>,
    next: HashMap<String, PathTable<R>>,
    wildcard: Option<Box<Wildcard<R>>>,
}

#[derive(Copy, Clone, PartialEq)]
enum WildcardKind {
    Segment,
    CatchAll,
}

impl std::fmt::Display for WildcardKind {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WildcardKind::Segment => Ok(()),
            WildcardKind::CatchAll => write!(fmt, "*"),
        }
    }
}

#[derive(Clone)]
struct Wildcard<R> {
    name: String,
    count_mod: WildcardKind,
    table: PathTable<R>,
}

/// For a successful match, this structure says how any wildcard segments were matched.
#[derive(Debug)]
pub struct RouteMatch<'a> {
    /// Wildcard matches in the order they appeared in the path.
    pub vec: Vec<&'a str>,
    /// Named wildcard matches, indexed by name.
    pub map: HashMap<&'a str, &'a str>,
}

impl<R> std::fmt::Debug for PathTable<R> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct Children<'a, R>(&'a HashMap<String, PathTable<R>>, Option<&'a Wildcard<R>>);
        impl<'a, R> std::fmt::Debug for Children<'a, R> {
            fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut dbg = fmt.debug_map();
                dbg.entries(self.0.iter());
                if let Some(wildcard) = self.1 {
                    dbg.entry(
                        &format_args!("{{{}}}{}", wildcard.name, wildcard.count_mod),
                        &wildcard.table,
                    );
                }
                dbg.finish()
            }
        }

        fmt.debug_struct("PathTable")
            .field(
                "resource",
                &format_args!(
                    "{}",
                    if self.accept.is_some() {
                        "Some"
                    } else {
                        "None"
                    }
                ),
            )
            .field(
                "children",
                &Children(&self.next, self.wildcard.as_ref().map(|x| &**x)),
            )
            .finish()
    }
}

impl<R> Default for PathTable<R> {
    fn default() -> Self {
        PathTable::new()
    }
}

impl<R> PathTable<R> {
    /// Create an empty routing table.
    pub fn new() -> PathTable<R> {
        PathTable {
            accept: None,
            next: HashMap::new(),
            wildcard: None,
        }
    }

    /// Get the resource of current path.
    pub fn resource(&self) -> Option<&R> {
        self.accept.as_ref()
    }

    /// Retrieve a mutable reference of the resource.
    pub fn resource_mut(&mut self) -> &mut Option<R> {
        &mut self.accept
    }

    /// Return an iterator of all resources.
    pub fn iter(&self) -> Resources<'_, R> {
        Resources { stack: vec![self] }
    }

    /// Return a mutable iterator of all resources.
    pub fn iter_mut(&mut self) -> ResourcesMut<'_, R> {
        ResourcesMut { stack: vec![self] }
    }

    /// Determine which resource, if any, the concrete `path` should be routed to.
    pub fn route<'a>(&'a self, path: &'a str) -> Option<(&'a R, RouteMatch<'a>)> {
        let mut table = self;
        let mut params = Vec::new();
        let mut param_map = HashMap::new();

        // Find all segments with their indices.
        let segment_iter = path
            .match_indices('/')
            .chain(std::iter::once((path.len(), "")))
            .scan(0usize, |prev_idx, (idx, _)| {
                let starts_at = *prev_idx;
                let segment = &path[starts_at..idx];
                *prev_idx = idx + 1;
                Some((starts_at, segment))
            });

        for (starts_at, mut segment) in segment_iter {
            if segment.is_empty() {
                continue;
            }

            if let Some(next_table) = table.next.get(segment) {
                table = next_table;
            } else if let Some(wildcard) = &table.wildcard {
                let last = if wildcard.count_mod == WildcardKind::CatchAll {
                    segment = &path[starts_at..];
                    true
                } else {
                    false
                };

                params.push(segment);

                if !wildcard.name.is_empty() {
                    param_map.insert(&*wildcard.name, segment);
                }

                table = &wildcard.table;

                if last {
                    break;
                }
            } else {
                return None;
            }
        }

        if table.accept.is_none() {
            if let Some(wildcard) = &table.wildcard {
                if wildcard.count_mod == WildcardKind::CatchAll {
                    params.push("");

                    if !wildcard.name.is_empty() {
                        param_map.insert(&*wildcard.name, "");
                    }

                    table = &wildcard.table;
                }
            }
        }

        table.accept.as_ref().map(|res| {
            (
                res,
                RouteMatch {
                    vec: params,
                    map: param_map,
                },
            )
        })
    }

    fn wildcard_mut(&mut self) -> Option<&mut Wildcard<R>> {
        self.wildcard.as_mut().map(|b| &mut **b)
    }

    /// Return the table of the given routing path (which may contain wildcards).
    ///
    /// If it doesn't already exist, this will make a new one.
    pub fn setup_table(&mut self, path: &str) -> &mut PathTable<R> {
        let mut table = self;
        let mut forbid_next = false;
        for segment in path.split('/') {
            if segment.is_empty() {
                continue;
            }

            if forbid_next {
                panic!("No segments are allowed after wildcard with `*` modifier");
            }

            let wildcard_opt = if segment.starts_with('{') {
                if segment.ends_with('}') {
                    Some((&segment[1..segment.len() - 1], WildcardKind::Segment))
                } else if segment.ends_with("}*") {
                    Some((&segment[1..segment.len() - 2], WildcardKind::CatchAll))
                } else {
                    None
                }
            } else if segment == "*" {
                Some(("", WildcardKind::CatchAll))
            } else {
                None
            };

            if let Some((name, count_mod)) = wildcard_opt {
                if count_mod != WildcardKind::Segment {
                    forbid_next = true;
                }

                if table.wildcard.is_none() {
                    table.wildcard = Some(Box::new(Wildcard {
                        name: name.to_string(),
                        count_mod,
                        table: PathTable::new(),
                    }));
                }

                match table.wildcard_mut().unwrap() {
                    Wildcard {
                        name: n,
                        count_mod: c,
                        ..
                    } if name != n || count_mod != *c => {
                        panic!(
                            "Route {} segment `{{{}}}{}` conflicts with existing wildcard segment `{{{}}}{}`",
                            path, name, count_mod, n, c
                        );
                    }
                    Wildcard { table: t, .. } => {
                        table = t;
                    }
                }
            } else {
                table = table
                    .next
                    .entry(segment.to_string())
                    .or_insert_with(PathTable::new);
            }
        }

        table
    }
}

impl<R: Default> PathTable<R> {
    /// Add or access a new resource at the given routing path (which may contain wildcards).
    pub fn setup(&mut self, path: &str) -> &mut R {
        let table = self.setup_table(path);

        if table.accept.is_none() {
            table.accept = Some(R::default())
        }

        table.accept.as_mut().unwrap()
    }
}

/// An iterator over the resources of a `PathTable`.
#[derive(Debug)]
pub struct Resources<'a, R> {
    stack: Vec<&'a PathTable<R>>,
}

impl<'a, R> Iterator for Resources<'a, R> {
    type Item = &'a R;

    fn next(&mut self) -> Option<&'a R> {
        while let Some(table) = self.stack.pop() {
            self.stack.extend(table.next.values());
            if let Some(wildcard) = table.wildcard.as_ref() {
                self.stack.push(&wildcard.table);
            }
            if let Some(res) = &table.accept {
                return Some(res);
            }
        }
        None
    }
}

/// A mutable iterator over the resources of a `PathTable`.
#[derive(Debug)]
pub struct ResourcesMut<'a, R> {
    stack: Vec<&'a mut PathTable<R>>,
}

impl<'a, R> Iterator for ResourcesMut<'a, R> {
    type Item = &'a mut R;

    fn next(&mut self) -> Option<&'a mut R> {
        while let Some(table) = self.stack.pop() {
            self.stack.extend(table.next.values_mut());
            if let Some(wildcard) = table.wildcard.as_mut() {
                self.stack.push(&mut wildcard.table);
            }
            if let Some(res) = &mut table.accept {
                return Some(res);
            }
        }
        None
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn empty_route_no_matches() {
        let table: PathTable<()> = PathTable::new();

        assert!(table.route("").is_none());
        assert!(table.route("/").is_none());
        assert!(table.route("//").is_none());
        assert!(table.route("foo").is_none());
        assert!(table.route("foo/bar").is_none());
    }

    #[test]
    fn root_matches() {
        let mut table: PathTable<()> = PathTable::new();
        table.setup("/");

        assert!(table.route("").is_some());
        assert!(table.route("/").is_some());
        assert!(table.route("//").is_some());

        assert!(table.route("foo").is_none());
        assert!(table.route("foo/bar").is_none());
    }

    #[test]
    fn one_fixed_segment_matches() {
        let mut table: PathTable<()> = PathTable::new();
        table.setup("foo");

        assert!(table.route("").is_none());
        assert!(table.route("/").is_none());
        assert!(table.route("//").is_none());

        assert!(table.route("foo").is_some());
        assert!(table.route("/foo").is_some());
        assert!(table.route("foo/").is_some());
        assert!(table.route("/foo/").is_some());
        assert!(table.route("//foo//").is_some());

        assert!(table.route("foo/bar").is_none());
        assert!(table.route("fo/o").is_none());
    }

    #[test]
    fn multiple_fixed_segment_matches() {
        let mut table: PathTable<()> = PathTable::new();
        table.setup("foo");
        table.setup("bar");

        assert!(table.route("").is_none());
        assert!(table.route("/").is_none());
        assert!(table.route("//").is_none());

        assert!(table.route("foo").is_some());
        assert!(table.route("bar").is_some());

        assert!(table.route("foo/bar").is_none());
        assert!(table.route("bar/foo").is_none())
    }

    #[test]
    fn nested_fixed_segment_matches() {
        let mut table: PathTable<()> = PathTable::new();
        table.setup("foo/bar");

        assert!(table.route("").is_none());
        assert!(table.route("foo").is_none());

        assert!(table.route("foo/bar").is_some());
    }

    #[test]
    fn multiple_nested_fixed_segment_matches() {
        let mut table: PathTable<()> = PathTable::new();
        table.setup("foo/bar");
        table.setup("baz");
        table.setup("quux/twiddle/twibble");

        assert!(table.route("").is_none());
        assert!(table.route("foo").is_none());
        assert!(table.route("quux").is_none());

        assert!(table.route("foo/bar").is_some());
        assert!(table.route("baz").is_some());
        assert!(table.route("quux/twiddle/twibble").is_some());
    }

    #[test]
    fn overlap_nested_fixed_segment_matches() {
        let mut table: PathTable<i32> = PathTable::new();
        *table.setup("") = 0;
        *table.setup("foo") = 1;
        *table.setup("foo/bar") = 2;

        assert_eq!(*table.route("/").unwrap().0, 0);
        assert_eq!(*table.route("/foo").unwrap().0, 1);
        assert_eq!(*table.route("/foo/bar").unwrap().0, 2);

        assert_eq!(*table.route("").unwrap().0, 0);
        assert_eq!(*table.route("foo").unwrap().0, 1);
        assert_eq!(*table.route("foo/bar").unwrap().0, 2);
    }

    #[test]
    fn wildcard_matches() {
        let mut table: PathTable<()> = PathTable::new();
        table.setup("{}");

        assert!(table.route("").is_none());
        assert!(table.route("foo/bar").is_none());

        assert!(table.route("foo").is_some());
        assert!(table.route("bar").is_some());
    }

    #[test]
    fn nested_wildcard_matches() {
        let mut table: PathTable<()> = PathTable::new();
        table.setup("{}/{}");

        assert!(table.route("").is_none());
        assert!(table.route("foo").is_none());

        assert!(table.route("foo/bar").is_some());
        assert_eq!(&table.route("foo/bar").unwrap().1.vec, &["foo", "bar"]);
        assert!(table.route("foo/bar").unwrap().1.map.is_empty());
    }

    #[test]
    fn mixed_route() {
        let mut table: PathTable<()> = PathTable::new();
        table.setup("foo/{}/bar");

        assert!(table.route("").is_none());
        assert!(table.route("foo").is_none());
        assert!(table.route("foo/bar").is_none());
        assert!(table.route("foo/bar/baz").is_none());

        assert!(table.route("foo/baz/bar").is_some());
        assert_eq!(&table.route("foo/baz/bar").unwrap().1.vec, &["baz"]);
    }

    #[test]
    fn wildcard_fallback() {
        let mut table: PathTable<i32> = PathTable::new();
        *table.setup("foo") = 0;
        *table.setup("foo/bar") = 1;
        *table.setup("foo/{}/bar") = 2;

        assert!(table.route("").is_none());
        assert!(table.route("foo/bar/baz").is_none());
        assert!(table.route("foo/bar/bar").is_none());

        assert_eq!(*table.route("foo").unwrap().0, 0);
        assert_eq!(*table.route("foo/bar").unwrap().0, 1);
        assert_eq!(*table.route("foo/baz/bar").unwrap().0, 2);
    }

    #[test]
    fn named_wildcard() {
        let mut table: PathTable<()> = PathTable::new();
        *table.setup("foo/{foo}");
        *table.setup("foo/{foo}/{bar}");
        *table.setup("{}");

        let (_, params) = table.route("foo/a").unwrap();
        assert_eq!(params.vec, &["a"]);
        assert_eq!(params.map, [("foo", "a")].iter().cloned().collect());

        let (_, params) = table.route("foo/a/b").unwrap();
        assert_eq!(params.vec, &["a", "b"]);
        assert_eq!(
            params.map,
            [("foo", "a"), ("bar", "b")].iter().cloned().collect()
        );

        let (_, params) = table.route("c").unwrap();
        assert_eq!(params.vec, &["c"]);
        assert!(params.map.is_empty());
    }

    #[test]
    fn wildcard_count_mod() {
        let mut table: PathTable<usize> = PathTable::new();
        *table.setup("foo/{foo}*") = 0;
        *table.setup("bar/{}*") = 1;
        *table.setup("baz/*") = 2;
        *table.setup("foo/bar") = 3;

        let (&id, params) = table.route("foo/a/b").unwrap();
        assert_eq!(id, 0);
        assert_eq!(params.vec, &["a/b"]);
        assert_eq!(params.map, [("foo", "a/b")].iter().cloned().collect());

        let (&id, params) = table.route("bar/a/b").unwrap();
        assert_eq!(id, 1);
        assert_eq!(params.vec, &["a/b"]);
        assert!(params.map.is_empty());

        let (&id, params) = table.route("baz/a/b").unwrap();
        assert_eq!(id, 2);
        assert_eq!(params.vec, &["a/b"]);
        assert!(params.map.is_empty());

        let (&id, params) = table.route("foo/bar").unwrap();
        assert_eq!(id, 3);
        assert!(params.vec.is_empty());
        assert!(params.map.is_empty());
    }

    #[test]
    #[should_panic]
    fn conflicting_wildcard_name_fails() {
        let mut table: PathTable<()> = PathTable::new();
        *table.setup("foo/{foo}");
        *table.setup("foo/{bar}");
    }

    #[test]
    #[should_panic]
    fn conflicting_wildcard_modifier_fails() {
        let mut table: PathTable<()> = PathTable::new();
        table.setup("foo/{foo}*");
        table.setup("foo/{foo}");
    }

    #[test]
    fn iter() {
        let mut table: PathTable<usize> = PathTable::new();
        *table.setup("foo") = 1;
        *table.setup("foo/bar") = 2;
        *table.setup("{}") = 3;

        let set: HashSet<_> = table.iter().cloned().collect();
        assert_eq!(set, [1, 2, 3].iter().cloned().collect());
    }

    #[test]
    fn iter_mut() {
        let mut table: PathTable<usize> = PathTable::new();
        *table.setup("foo") = 1;
        *table.setup("foo/bar") = 2;
        *table.setup("{}") = 3;

        for res in table.iter_mut() {
            *res -= 1;
        }

        let set: HashSet<_> = table.iter().cloned().collect();
        assert_eq!(set, [0, 1, 2].iter().cloned().collect());
    }
}
