use crate::{query_target::Matcher, util};
use graphql_parser::{
    query::{
        parse_query, Definition as QueryDef, Document as QueryDoc, Field as QueryField,
        FragmentDefinition, InlineFragment, Selection, SelectionSet, TypeCondition,
    },
    schema::parse_schema,
};
use std::collections::{HashMap, HashSet};

/// Strips a query down to just the selections needed to reach `targets`.
///
/// Each target is either a type name (`MyType`) or a field on a type
/// (`MyType.field`). Every path from an operation root down to a matching
/// selection is retained, along with the full sub-selection at the match, so the
/// result is a valid query rooted at the same entrypoints as the original.
/// Operations, fragments and variable definitions that no longer contribute
/// anything are dropped; if nothing matches, the result is empty.
pub fn process(schema: &str, query: &str, targets: &[&str]) -> String {
    // An empty document is what this command emits when nothing matches, so it
    // has to survive being piped back in rather than failing to parse.
    if query.trim().is_empty() {
        return String::new();
    }

    let schema_doc = parse_schema::<String>(schema).expect("Failed to parse schema");
    let query_doc = parse_query::<String>(query).expect("Failed to parse query");

    let focus = Focus {
        matcher: Matcher::new(&schema_doc, targets),
        fragments: query_doc
            .definitions
            .iter()
            .filter_map(|def| match def {
                QueryDef::Fragment(f) => Some((f.name.clone(), f)),
                _ => None,
            })
            .collect(),
    };

    let root_types = util::detect_root_types(&schema_doc);
    let mut walk = Walk::default();

    // Prune every operation first: a fragment may be reached from several
    // operations, and its retained shape must be settled before we can tell
    // which variables the document still uses.
    let mut pruned_ops: HashMap<usize, SelectionSet<String>> = HashMap::new();
    for (i, def) in query_doc.definitions.iter().enumerate() {
        if let QueryDef::Operation(op) = def {
            let (root_type, selection_set) = util::operation_root(&root_types, op);
            if let Some(pruned) = focus.focus_selection_set(root_type, selection_set, &mut walk) {
                pruned_ops.insert(i, pruned);
            }
        }
    }

    let definitions: Vec<_> = query_doc
        .definitions
        .iter()
        .enumerate()
        .filter_map(|(i, def)| match def {
            QueryDef::Operation(op) => {
                let selection_set = pruned_ops.remove(&i)?;
                let directives = util::operation_directives(op);
                let mut used_variables = HashSet::new();
                util::collect_directive_variables(directives, &mut used_variables);
                focus.collect_variables(
                    &selection_set,
                    &walk,
                    &mut HashSet::new(),
                    &mut used_variables,
                );
                Some(QueryDef::Operation(util::rebuild_operation(
                    op,
                    selection_set,
                    directives.to_vec(),
                    &used_variables,
                )))
            }
            // A fragment spread from inside a verbatim-retained selection is
            // printed in full; otherwise only its pruned form is kept.
            QueryDef::Fragment(frag) if walk.whole.contains(&frag.name) => {
                Some(QueryDef::Fragment(frag.clone()))
            }
            QueryDef::Fragment(frag) => walk.retained(&frag.name).cloned().map(QueryDef::Fragment),
        })
        .collect();

    if definitions.is_empty() {
        return String::new();
    }

    format!("{}", QueryDoc { definitions })
}

/// The result of pruning one fragment definition against the targets.
struct FragmentOutcome<'q> {
    /// Whether spreading this fragment reaches a target, which is what decides
    /// whether a spread sitting on a path is retained.
    ///
    /// Deliberately independent of `Walk::whole`: a fragment spread inside a
    /// matched sub-selection is emitted verbatim, but that says nothing about
    /// whether the fragment reaches a target *itself*. Conflating the two let a
    /// fragment retained by one branch drag in every sibling branch that spread
    /// it, keeping interface implementations that reach nothing.
    reaches_target: bool,
    /// The reduced definition, when the fragment reaches a target on its own.
    pruned: Option<FragmentDefinition<'q, String>>,
}

/// Mutable fragment bookkeeping threaded through the walk.
#[derive(Default)]
struct Walk<'q> {
    outcomes: HashMap<String, FragmentOutcome<'q>>,
    /// Fragments currently being resolved, to break illegal fragment cycles.
    resolving: HashSet<String>,
    /// Fragments that must be emitted verbatim, because they are spread from
    /// inside a selection that was retained whole.
    whole: HashSet<String>,
}

impl<'q> Walk<'q> {
    /// The retained form of a fragment, or `None` if it was dropped.
    fn retained(&self, name: &str) -> Option<&FragmentDefinition<'q, String>> {
        self.outcomes.get(name).and_then(|o| o.pruned.as_ref())
    }
}

struct Focus<'s, 'q, 't> {
    matcher: Matcher<'s, 't>,
    fragments: HashMap<String, &'q FragmentDefinition<'q, String>>,
}

impl<'s, 'q> Focus<'s, 'q, '_> {
    /// Prunes `selection_set` (selected on `parent_type`) to the paths that
    /// reach a target, returning `None` when none do.
    fn focus_selection_set(
        &self,
        parent_type: &str,
        selection_set: &SelectionSet<'q, String>,
        walk: &mut Walk<'q>,
    ) -> Option<SelectionSet<'q, String>> {
        let mut items = Vec::new();

        for selection in &selection_set.items {
            match selection {
                Selection::Field(field) => {
                    let field_type = self.matcher.field_type(parent_type, &field.name);

                    if self.matcher.is_field_target(parent_type, &field.name)
                        || field_type.is_some_and(|ty| self.matcher.is_type_target(ty))
                    {
                        self.retain_fragments_whole(&field.selection_set, walk);
                        items.push(selection.clone());
                    } else if let Some(sub) = field_type
                        .and_then(|ty| self.focus_selection_set(ty, &field.selection_set, walk))
                    {
                        items.push(Selection::Field(QueryField {
                            selection_set: sub,
                            ..field.clone()
                        }));
                    }
                }
                Selection::FragmentSpread(spread) => {
                    if self.focus_fragment(&spread.fragment_name, walk) {
                        items.push(selection.clone());
                    }
                }
                Selection::InlineFragment(inline) => {
                    let condition = util::type_condition(inline.type_condition.as_ref());

                    if condition.is_some_and(|ty| self.matcher.is_type_target(ty)) {
                        self.retain_fragments_whole(&inline.selection_set, walk);
                        items.push(selection.clone());
                    } else if let Some(sub) = self.focus_selection_set(
                        condition.unwrap_or(parent_type),
                        &inline.selection_set,
                        walk,
                    ) {
                        items.push(Selection::InlineFragment(InlineFragment {
                            selection_set: sub,
                            ..inline.clone()
                        }));
                    }
                }
            }
        }

        if items.is_empty() {
            None
        } else {
            Some(SelectionSet {
                span: selection_set.span,
                items,
            })
        }
    }

    /// Resolves a fragment spread, recording how much of the fragment is kept.
    /// Returns whether spreading it reaches a target, i.e. whether the spread
    /// itself should be retained.
    fn focus_fragment(&self, name: &str, walk: &mut Walk<'q>) -> bool {
        if let Some(outcome) = walk.outcomes.get(name) {
            return outcome.reaches_target;
        }
        if !walk.resolving.insert(name.to_string()) {
            // Only reachable through an illegal fragment cycle; break it.
            return false;
        }

        let outcome = match self.fragments.get(name).copied() {
            None => FragmentOutcome {
                reaches_target: false,
                pruned: None,
            },
            Some(fragment) => {
                let TypeCondition::On(condition) = &fragment.type_condition;

                if self.matcher.is_type_target(condition) {
                    // The fragment's own type is targeted, so it is a match and
                    // is kept verbatim along with everything it spreads.
                    self.retain_fragment_whole(name, walk);
                    FragmentOutcome {
                        reaches_target: true,
                        pruned: None,
                    }
                } else {
                    let pruned = self
                        .focus_selection_set(condition, &fragment.selection_set, walk)
                        .map(|selection_set| FragmentDefinition {
                            selection_set,
                            ..fragment.clone()
                        });

                    FragmentOutcome {
                        reaches_target: pruned.is_some(),
                        pruned,
                    }
                }
            }
        };

        walk.resolving.remove(name);
        let reaches_target = outcome.reaches_target;
        walk.outcomes.insert(name.to_string(), outcome);
        reaches_target
    }

    /// Marks a fragment, and everything it spreads, as emitted verbatim.
    fn retain_fragment_whole(&self, name: &str, walk: &mut Walk<'q>) {
        if !walk.whole.insert(name.to_string()) {
            return;
        }
        if let Some(fragment) = self.fragments.get(name).copied() {
            self.retain_fragments_whole(&fragment.selection_set, walk);
        }
    }

    /// Marks every fragment reachable from a verbatim-retained selection set as
    /// emitted verbatim, since its spread sites are kept untouched.
    ///
    /// This only affects how those fragments are *printed*. It must not make
    /// them look like they reach a target, or spreads of the same fragment
    /// elsewhere would be retained on paths that reach nothing.
    fn retain_fragments_whole(
        &self,
        selection_set: &SelectionSet<'q, String>,
        walk: &mut Walk<'q>,
    ) {
        for selection in &selection_set.items {
            match selection {
                Selection::Field(field) => self.retain_fragments_whole(&field.selection_set, walk),
                Selection::InlineFragment(inline) => {
                    self.retain_fragments_whole(&inline.selection_set, walk)
                }
                Selection::FragmentSpread(spread) => {
                    self.retain_fragment_whole(&spread.fragment_name, walk)
                }
            }
        }
    }

    /// Collects the names of every variable still referenced by the retained
    /// document, following fragment spreads into their retained definitions.
    fn collect_variables(
        &self,
        selection_set: &SelectionSet<'q, String>,
        walk: &Walk<'q>,
        seen_fragments: &mut HashSet<String>,
        used: &mut HashSet<String>,
    ) {
        for selection in &selection_set.items {
            match selection {
                Selection::Field(field) => {
                    for (_name, value) in &field.arguments {
                        util::collect_value_variables(value, used);
                    }
                    util::collect_directive_variables(&field.directives, used);
                    self.collect_variables(&field.selection_set, walk, seen_fragments, used);
                }
                Selection::InlineFragment(inline) => {
                    util::collect_directive_variables(&inline.directives, used);
                    self.collect_variables(&inline.selection_set, walk, seen_fragments, used);
                }
                Selection::FragmentSpread(spread) => {
                    util::collect_directive_variables(&spread.directives, used);
                    if !seen_fragments.insert(spread.fragment_name.clone()) {
                        continue;
                    }
                    let retained = if walk.whole.contains(&spread.fragment_name) {
                        self.fragments.get(&spread.fragment_name).copied()
                    } else {
                        walk.retained(&spread.fragment_name)
                    };
                    if let Some(fragment) = retained {
                        util::collect_directive_variables(&fragment.directives, used);
                        self.collect_variables(&fragment.selection_set, walk, seen_fragments, used);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::query_focus;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    const SCHEMA: &str = indoc! {"
        schema {
          query: Query
        }

        type Query {
          user: User
          company: Company
          node(id: ID!): Node
        }

        interface Node {
          id: ID!
        }

        type User implements Node {
          id: ID!
          name: String
          profile: Profile
          manager: User
        }

        type Profile {
          email: String
          avatar: Avatar
        }

        type Avatar {
          url: String
        }

        type Company implements Node {
          id: ID!
          name: String
          employees(first: Int): [User]
        }
    "};

    #[test]
    fn keeps_path_and_subtree_for_a_type_target() {
        let query = indoc! {"
            query Q {
              user {
                name
                profile {
                  email
                  avatar {
                    url
                  }
                }
              }
              company {
                name
              }
            }
        "};

        let result = query_focus::process(SCHEMA, query, &["Profile"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  user {
                    profile {
                      email
                      avatar {
                        url
                      }
                    }
                  }
                }
            "}
        );
    }

    #[test]
    fn keeps_only_the_targeted_field() {
        let query = indoc! {"
            query Q {
              user {
                id
                name
                profile {
                  email
                }
              }
            }
        "};

        let result = query_focus::process(SCHEMA, query, &["User.name"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  user {
                    name
                  }
                }
            "}
        );
    }

    #[test]
    fn keeps_every_path_to_the_target() {
        let query = indoc! {"
            query Q {
              user {
                name
                manager {
                  name
                }
              }
              company {
                employees {
                  name
                }
              }
            }
        "};

        let result = query_focus::process(SCHEMA, query, &["User.name"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  user {
                    name
                    manager {
                      name
                    }
                  }
                  company {
                    employees {
                      name
                    }
                  }
                }
            "}
        );
    }

    #[test]
    fn matches_interface_field_on_implementing_type() {
        let query = indoc! {"
            query Q {
              user {
                id
                name
              }
              company {
                name
              }
            }
        "};

        let result = query_focus::process(SCHEMA, query, &["Node.id"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  user {
                    id
                  }
                }
            "}
        );
    }

    #[test]
    fn matches_inline_fragment_type_condition() {
        let query = indoc! {"
            query Q {
              node(id: \"1\") {
                id
                ... on User {
                  name
                  profile {
                    email
                  }
                }
                ... on Company {
                  name
                }
              }
            }
        "};

        let result = query_focus::process(SCHEMA, query, &["User"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  node(id: \"1\") {
                    ... on User {
                      name
                      profile {
                        email
                      }
                    }
                  }
                }
            "}
        );
    }

    #[test]
    fn prunes_fragments_and_drops_unreached_ones() {
        let query = indoc! {"
            query Q {
              user {
                ...UserFields
              }
              company {
                ...CompanyFields
              }
            }

            fragment UserFields on User {
              id
              name
              profile {
                email
              }
            }

            fragment CompanyFields on Company {
              id
              name
            }
        "};

        let result = query_focus::process(SCHEMA, query, &["Profile"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  user {
                    ...UserFields
                  }
                }

                fragment UserFields on User {
                  profile {
                    email
                  }
                }
            "}
        );
    }

    #[test]
    fn keeps_fragments_spread_inside_a_retained_subtree() {
        let query = indoc! {"
            query Q {
              user {
                name
                profile {
                  ...ProfileFields
                }
              }
            }

            fragment ProfileFields on Profile {
              email
              avatar {
                ...AvatarFields
              }
            }

            fragment AvatarFields on Avatar {
              url
            }
        "};

        let result = query_focus::process(SCHEMA, query, &["User.profile"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  user {
                    profile {
                      ...ProfileFields
                    }
                  }
                }

                fragment ProfileFields on Profile {
                  email
                  avatar {
                    ...AvatarFields
                  }
                }

                fragment AvatarFields on Avatar {
                  url
                }
            "}
        );
    }

    #[test]
    fn drops_variables_that_are_no_longer_referenced() {
        let query = indoc! {"
            query Q($id: ID!, $first: Int) {
              node(id: $id) {
                id
              }
              company {
                employees(first: $first) {
                  name
                }
              }
            }
        "};

        let result = query_focus::process(SCHEMA, query, &["User.name"]);

        assert_eq!(
            result,
            indoc! {"
                query Q($first: Int) {
                  company {
                    employees(first: $first) {
                      name
                    }
                  }
                }
            "}
        );
    }

    #[test]
    fn drops_operations_that_reach_nothing() {
        let query = indoc! {"
            query WithProfile {
              user {
                profile {
                  email
                }
              }
            }

            query WithoutProfile {
              company {
                name
              }
            }
        "};

        let result = query_focus::process(SCHEMA, query, &["Profile"]);

        assert_eq!(
            result,
            indoc! {"
                query WithProfile {
                  user {
                    profile {
                      email
                    }
                  }
                }
            "}
        );
    }

    #[test]
    fn returns_empty_when_nothing_matches() {
        let query = indoc! {"
            query Q {
              user {
                name
              }
            }
        "};

        assert_eq!(query_focus::process(SCHEMA, query, &["NoSuchType"]), "");
        assert_eq!(query_focus::process(SCHEMA, query, &["User.missing"]), "");
    }

    /// The empty document this command emits when nothing matches has to survive
    /// being piped back in.
    #[test]
    fn passes_an_empty_document_through() {
        assert_eq!(query_focus::process(SCHEMA, "", &["User"]), "");
        assert_eq!(query_focus::process(SCHEMA, "\n", &["User"]), "");
    }

    const INTERFACE_SCHEMA: &str = indoc! {"
        type Query {
          node: Node
        }

        interface Node {
          id: ID!
        }

        type A implements Node {
          id: ID!
          image: Image
        }

        type B implements Node {
          id: ID!
          thumb: Image
        }

        type Image {
          url: String
        }
    "};

    #[test]
    fn drops_sibling_inline_fragments_that_do_not_reach_the_target() {
        let query = indoc! {"
            query Q {
              node {
                ... on A {
                  image {
                    ...ImageFields
                  }
                }
                ... on B {
                  thumb {
                    ...ImageFields
                  }
                }
              }
            }

            fragment ImageFields on Image {
              url
            }
        "};

        let result = query_focus::process(INTERFACE_SCHEMA, query, &["A"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  node {
                    ... on A {
                      image {
                        ...ImageFields
                      }
                    }
                  }
                }

                fragment ImageFields on Image {
                  url
                }
            "}
        );
    }

    #[test]
    fn drops_sibling_spreads_that_do_not_reach_the_target() {
        let query = indoc! {"
            query Q {
              node {
                ...AFields
                ...BFields
              }
            }

            fragment AFields on A {
              image {
                ...ImageFields
              }
            }

            fragment BFields on B {
              thumb {
                ...ImageFields
              }
            }

            fragment ImageFields on Image {
              url
            }
        "};

        let result = query_focus::process(INTERFACE_SCHEMA, query, &["A"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  node {
                    ...AFields
                  }
                }

                fragment AFields on A {
                  image {
                    ...ImageFields
                  }
                }

                fragment ImageFields on Image {
                  url
                }
            "}
        );
    }

    /// The same shared fragment reached from two operations: the one that does
    /// not reach a target must not be dragged in by the one that does.
    #[test]
    fn shared_fragment_does_not_leak_across_operations() {
        let query = indoc! {"
            query Reaches {
              node {
                ... on A {
                  image {
                    ...ImageFields
                  }
                }
              }
            }

            query DoesNotReach {
              node {
                ... on B {
                  thumb {
                    ...ImageFields
                  }
                }
              }
            }

            fragment ImageFields on Image {
              url
            }
        "};

        let result = query_focus::process(INTERFACE_SCHEMA, query, &["A"]);

        assert_eq!(
            result,
            indoc! {"
                query Reaches {
                  node {
                    ... on A {
                      image {
                        ...ImageFields
                      }
                    }
                  }
                }

                fragment ImageFields on Image {
                  url
                }
            "}
        );
    }

    #[test]
    fn keeps_multiple_targets() {
        let query = indoc! {"
            query Q {
              user {
                id
                name
                profile {
                  email
                }
              }
              company {
                id
                name
              }
            }
        "};

        let result = query_focus::process(SCHEMA, query, &["Profile", "Company.name"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  user {
                    profile {
                      email
                    }
                  }
                  company {
                    name
                  }
                }
            "}
        );
    }
}
