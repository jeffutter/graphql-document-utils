//! Apollo Federation supergraphs: recognizing one, and knowing which of its
//! parts only federation reads.
//!
//! A supergraph is the schema composition produces from a set of subgraphs.
//! Its `schema` definition links the specs it uses with `@link`, and it records
//! which subgraph ("graph") has each type and field in the directives of the
//! join spec, such as `@join__type` and `@join__field`. Everything the linked
//! specs define, the machinery, is for the router, not for clients, and the
//! string arguments naming fields (`key: "id"`, `requires: "weight"`) are
//! references a plain schema does not have. The commands that trim a schema
//! consult this module so a trimmed supergraph is still one, the query
//! commands resolve against its API schema, and `subgraph` extracts from it.
//!
//! Everything here follows the public link and join specs
//! (<https://specs.apollo.dev>), written against graphql-parser.

use crate::util;
use graphql_parser::{
    query::{self, Selection, SelectionSet, Value},
    schema::{
        Definition, Directive, Document, EnumType, Field, InputObjectType, InputValue,
        InterfaceType, ObjectType, ScalarType, Type, TypeDefinition, UnionType,
    },
};
use std::collections::{BTreeSet, HashMap, HashSet};

/// A spec the schema links to, as `@link(url:
/// "https://specs.apollo.dev/join/v0.4")` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feature {
    /// The spec's name, the second-to-last segment of its URL (`join`).
    pub name: String,
    /// The version, the last segment (`v0.4`), as `(major, minor)`.
    pub version: (u32, u32),
    /// The URL as written.
    pub url: String,
    /// What the schema prefixes the spec's elements with: `as:` if given, else
    /// the spec's name, so `@join__type` and `join__Graph`.
    prefix: String,
    /// Each imported element as the spec names it and as the schema does,
    /// which `import: [{ name: "@key", as: "@primaryKey" }]` makes differ.
    /// Directives keep their `@`.
    imports: Vec<(String, String)>,
}

impl Feature {
    /// Reads the feature a `@link` (or Federation 1's `@core`) application
    /// links, or `None` when it has no URL in the form
    /// `<anything>/<name>/v<major>.<minor>`.
    fn from_link(directive: &Directive<'_, String>) -> Option<Feature> {
        let url = string_argument(directive, "url").or(string_argument(directive, "feature"))?;
        let path = url.split(['?', '#']).next()?.trim_end_matches('/');
        let (rest, version) = path.rsplit_once('/')?;
        let name = rest.rsplit('/').next()?;
        let (major, minor) = version.strip_prefix('v')?.split_once('.')?;
        let version = (major.parse().ok()?, minor.parse().ok()?);
        if name.is_empty() {
            return None;
        }

        let imports = match argument(directive, "import") {
            Some(Value::List(items)) => items
                .iter()
                .filter_map(|item| match item {
                    Value::String(name) => Some((name.clone(), name.clone())),
                    Value::Object(fields) => {
                        let name = match fields.get("name") {
                            Some(Value::String(name)) => name.clone(),
                            _ => return None,
                        };
                        let local = match fields.get("as") {
                            Some(Value::String(local)) => local.clone(),
                            _ => name.clone(),
                        };
                        Some((name, local))
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };

        Some(Feature {
            name: name.to_string(),
            version,
            url: url.to_string(),
            prefix: string_argument(directive, "as").unwrap_or(name).to_string(),
            imports,
        })
    }

    /// The name the schema gives the spec's directive `element`: the spec's
    /// own name for its main directive (`@inaccessible`), else `type` for
    /// `@join__type`.
    fn directive(&self, element: &str) -> String {
        if element == self.name {
            return self.prefix.clone();
        }
        let original = format!("@{element}");
        match self.imports.iter().find(|(name, _)| *name == original) {
            Some((_, local)) => local.trim_start_matches('@').to_string(),
            None => format!("{}__{element}", self.prefix),
        }
    }

    /// Which of the spec's directives `name` is, as the spec names it, or
    /// `None` when it is not one of them. The inverse of `directive`.
    fn directive_element(&self, name: &str) -> Option<String> {
        if name == self.prefix {
            return Some(self.name.clone());
        }
        let imported = self
            .imports
            .iter()
            .find(|(_, local)| local.strip_prefix('@') == Some(name));
        match imported {
            Some((original, _)) => Some(original.trim_start_matches('@').to_string()),
            None => name
                .strip_prefix(&self.prefix)
                .and_then(|rest| rest.strip_prefix("__"))
                .map(str::to_string),
        }
    }

    /// Whether the type `name` is one of the spec's.
    fn defines_type(&self, name: &str) -> bool {
        self.imports.iter().any(|(_, local)| local == name)
            || name
                .strip_prefix(&self.prefix)
                .is_some_and(|rest| rest.starts_with("__"))
    }
}

/// A subgraph, as a value of the `join__Graph` enum records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    /// The enum value, which the join directives name it by (`ACCOUNTS`).
    pub value: String,
    /// The subgraph's own name, from `@join__graph(name:)` (`accounts`).
    pub name: String,
}

/// A selection of fields a join directive names in a string, which a
/// trimmed supergraph must keep, since the router selects them.
#[derive(Debug, Clone)]
pub struct FieldSet {
    /// The type declaring it.
    pub type_name: String,
    /// The field it is on, or `None` for a key, which is on the type.
    pub field: Option<String>,
    /// The type the selection is made on: the type itself for a key or
    /// `requires`, the field's return type for `provides`.
    pub on: String,
    pub selection: SelectionSet<'static, String>,
}

/// What a schema's `@link`s say about it, when it is a supergraph.
#[derive(Debug, Clone)]
pub struct Supergraph {
    /// The name of the directive that links features, `link`, or `core` in a
    /// Federation 1 supergraph.
    link: String,
    features: Vec<Feature>,
    /// Which of `features` is the join spec.
    join: usize,
}

/// The federation directives a subgraph can apply that composition carries
/// into the supergraph as a spec of their own, by that spec's name and the
/// directive's. Extraction copies them back (see `Supergraph::carried`).
const CARRIED: [(&str, &str); 7] = [
    ("inaccessible", "inaccessible"),
    ("tag", "tag"),
    ("authenticated", "authenticated"),
    ("requiresScopes", "requiresScopes"),
    ("policy", "policy"),
    ("cost", "cost"),
    ("cost", "listSize"),
];

impl Supergraph {
    /// Recognizes a supergraph: a schema whose `schema` definition links the
    /// join spec. `None` for any other schema.
    ///
    /// The directive that links specs names itself: its application whose URL
    /// is the link spec's (or Federation 1's core spec) says what it is
    /// called, so a schema that renames `@link` is still read.
    pub fn detect(doc: &Document<'_, String>) -> Option<Supergraph> {
        let sd = doc.definitions.iter().find_map(|def| match def {
            Definition::SchemaDefinition(sd) => Some(sd),
            _ => None,
        })?;
        let link = sd
            .directives
            .iter()
            .find(|d| Feature::from_link(d).is_some_and(|f| f.name == "link" || f.name == "core"))?
            .name
            .clone();
        let features: Vec<Feature> = sd
            .directives
            .iter()
            .filter(|d| d.name == link)
            .filter_map(Feature::from_link)
            .collect();
        let join = features.iter().position(|f| f.name == "join")?;
        Some(Supergraph {
            link,
            features,
            join,
        })
    }

    /// Whether this is a Federation 1 supergraph: linked with `@core`, or
    /// using join v0.1, whose directives record less than later versions.
    pub fn is_federation_1(&self) -> bool {
        self.features.iter().any(|f| f.name == "core") || self.join_spec().version < (0, 2)
    }

    /// The join spec's URL, for messages.
    pub fn join_url(&self) -> &str {
        &self.join_spec().url
    }

    fn join_spec(&self) -> &Feature {
        &self.features[self.join]
    }

    /// The name the schema gives the join directive `element` (`type` for
    /// `@join__type`).
    pub fn join(&self, element: &str) -> String {
        self.join_spec().directive(element)
    }

    /// The applications of the join directive `element` among `directives`.
    pub fn joins<'d, 'a>(
        &self,
        directives: &'d [Directive<'a, String>],
        element: &str,
    ) -> Vec<&'d Directive<'a, String>> {
        let name = self.join(element);
        directives.iter().filter(|d| d.name == name).collect()
    }

    /// Whether the directive `name` belongs to a linked spec, such as
    /// `@link`, `@join__type`, or `@inaccessible`.
    pub fn is_machinery_directive(&self, name: &str) -> bool {
        name == self.link
            || self
                .features
                .iter()
                .any(|f| f.directive_element(name).is_some())
    }

    /// Whether the type `name` belongs to a linked spec, such as
    /// `join__Graph` or `link__Purpose`.
    pub fn is_machinery_type(&self, name: &str) -> bool {
        self.features.iter().any(|f| f.defines_type(name))
    }

    /// The spec a machinery directive `name` comes from, and which of its
    /// directives it is.
    pub fn machinery_directive(&self, name: &str) -> Option<(&Feature, String)> {
        self.features
            .iter()
            .find_map(|f| f.directive_element(name).map(|element| (f, element)))
    }

    /// The federation directive a subgraph writes for the machinery directive
    /// `name`, when it is one composition carries through as a spec of its
    /// own: `@inaccessible`, `@tag`, and the rest of `CARRIED`.
    pub fn carried(&self, name: &str) -> Option<&'static str> {
        let (feature, element) = self.machinery_directive(name)?;
        CARRIED
            .iter()
            .find(|(spec, directive)| *spec == feature.name && *directive == element)
            .map(|(_, directive)| *directive)
    }

    /// The subgraphs, in the order the `join__Graph` enum lists them.
    pub fn graphs(&self, types: &HashMap<String, TypeDefinition<'_, String>>) -> Vec<Graph> {
        let Some(TypeDefinition::Enum(graphs)) =
            types.get(&format!("{}__Graph", self.join_spec().prefix))
        else {
            return Vec::new();
        };
        graphs
            .values
            .iter()
            .map(|value| Graph {
                value: value.name.clone(),
                name: self
                    .joins(&value.directives, "graph")
                    .first()
                    .and_then(|d| string_argument(d, "name"))
                    .unwrap_or(&value.name)
                    .to_string(),
            })
            .collect()
    }

    /// Every field set the join directives of `types` name, parsed, in type
    /// and field order: each key on its type, each `requires` on the type
    /// declaring the field, and each `provides` on the type the field returns
    /// in that graph. `Err` holds the message for one that does not parse as
    /// a selection set.
    pub fn field_sets(
        &self,
        types: &HashMap<String, TypeDefinition<'_, String>>,
    ) -> Result<Vec<FieldSet>, String> {
        let mut names: Vec<&String> = types.keys().collect();
        names.sort();
        let mut sets = Vec::new();
        for name in names {
            let (directives, fields) = match &types[name] {
                TypeDefinition::Object(obj) => (&obj.directives, &obj.fields),
                TypeDefinition::Interface(iface) => (&iface.directives, &iface.fields),
                _ => continue,
            };
            for key in self.joins(directives, "type") {
                if let Some(text) = string_argument(key, "key") {
                    sets.push(FieldSet {
                        type_name: name.clone(),
                        field: None,
                        on: name.clone(),
                        selection: parse_field_set(text)
                            .ok_or_else(|| not_a_selection("key", text, name))?,
                    });
                }
            }
            for field in fields {
                let owner = format!("{name}.{}", field.name);
                for join in self.joins(&field.directives, "field") {
                    if let Some(text) = string_argument(join, "requires") {
                        sets.push(FieldSet {
                            type_name: name.clone(),
                            field: Some(field.name.clone()),
                            on: name.clone(),
                            selection: parse_field_set(text)
                                .ok_or_else(|| not_a_selection("requires", text, &owner))?,
                        });
                    }
                    if let Some(text) = string_argument(join, "provides") {
                        let returns = string_argument(join, "type")
                            .and_then(parse_type)
                            .and_then(|ty| util::named_type(&ty).cloned())
                            .or_else(|| util::named_type(&field.field_type).cloned());
                        let Some(returns) = returns else { continue };
                        sets.push(FieldSet {
                            type_name: name.clone(),
                            field: Some(field.name.clone()),
                            on: returns,
                            selection: parse_field_set(text)
                                .ok_or_else(|| not_a_selection("provides", text, &owner))?,
                        });
                    }
                }
            }
        }
        Ok(sets)
    }

    /// The types a field's (or input field's) `@join__field(type:)`
    /// applications name: the type it has in a graph where it differs from
    /// the supergraph's, which the schema must still define.
    pub fn graph_types(&self, directives: &[Directive<'_, String>]) -> Vec<String> {
        self.joins(directives, "field")
            .into_iter()
            .filter_map(|d| string_argument(d, "type"))
            .filter_map(parse_type)
            .filter_map(|ty| util::named_type(&ty).cloned())
            .collect()
    }

    /// Every type the join directives of `td` name by string: the graph types
    /// of its fields (see `graph_types`).
    pub fn named_types(&self, td: &TypeDefinition<'_, String>) -> Vec<String> {
        match td {
            TypeDefinition::Object(ObjectType { fields, .. })
            | TypeDefinition::Interface(InterfaceType { fields, .. }) => fields
                .iter()
                .flat_map(|f| self.graph_types(&f.directives))
                .collect(),
            TypeDefinition::InputObject(input) => input
                .fields
                .iter()
                .flat_map(|f| self.graph_types(&f.directives))
                .collect(),
            TypeDefinition::Scalar(_) | TypeDefinition::Union(_) | TypeDefinition::Enum(_) => {
                Vec::new()
            }
        }
    }

    /// The graphs that have `td`: those its `@join__type` applications name,
    /// or every graph when it has none, as a type composition saw in every
    /// subgraph (or none of the specs' own) needs none.
    pub fn type_graphs(&self, td: &TypeDefinition<'_, String>, graphs: &[Graph]) -> Vec<String> {
        let joins = self.joins(type_directives(td), "type");
        if joins.is_empty() {
            return graphs.iter().map(|g| g.value.clone()).collect();
        }
        let mut named: Vec<String> = Vec::new();
        for graph in joins.iter().filter_map(|d| enum_argument(d, "graph")) {
            if !named.iter().any(|g| g == graph) {
                named.push(graph.to_string());
            }
        }
        named
    }

    /// The `@join__field` applications a field (or input field) with
    /// `directives` has for `graph`, or `None` when the field is not in that
    /// graph. A field with none is in every graph its type is in, so it is
    /// in `graph` when `type_in_graph` says so. A field with some is in the
    /// graphs they name; one naming no graph puts it in none, as composition
    /// writes for a field only an `@interfaceObject` contributes.
    pub fn field_joins<'d, 'a>(
        &self,
        directives: &'d [Directive<'a, String>],
        type_in_graph: bool,
        graph: &str,
    ) -> Option<Vec<&'d Directive<'a, String>>> {
        let joins = self.joins(directives, "field");
        if joins.is_empty() {
            return type_in_graph.then(Vec::new);
        }
        let own: Vec<_> = joins
            .into_iter()
            .filter(|d| enum_argument(d, "graph") == Some(graph))
            .collect();
        (!own.is_empty()).then_some(own)
    }

    /// Whether the field with `directives` is in `graph` and resolved there,
    /// not only declared `@external` to it.
    pub fn resolves_in(
        &self,
        directives: &[Directive<'_, String>],
        type_in_graph: bool,
        graph: &str,
    ) -> bool {
        self.field_joins(directives, type_in_graph, graph)
            .is_some_and(|joins| !joins.iter().any(|d| is_true(d, "external")))
    }

    /// The type a field has in `graph`: its `@join__field(type:)` there,
    /// else its type in the supergraph.
    pub fn type_in<'a>(&self, field: &Field<'a, String>, graph: &str) -> Type<'a, String> {
        self.joins(&field.directives, "field")
            .into_iter()
            .filter(|d| enum_argument(d, "graph") == Some(graph))
            .find_map(|d| string_argument(d, "type").and_then(parse_type))
            .unwrap_or_else(|| field.field_type.clone())
    }

    /// The fields `field_set` names, by type and field, following nested
    /// selections into the types their fields return in `graph` (or in the
    /// supergraph, when `None`), and inline fragments into the types they
    /// name. A field `types` does not define is named all the same, but
    /// nothing below it is followed.
    pub fn field_set_fields(
        &self,
        types: &HashMap<String, TypeDefinition<'_, String>>,
        field_set: &FieldSet,
        graph: Option<&str>,
    ) -> HashSet<(String, String)> {
        let mut named = HashSet::new();
        walk_field_set(
            self,
            types,
            &field_set.on,
            &field_set.selection,
            graph,
            &mut named,
        );
        named
    }

    /// Brings the join directives on a trimmed `td` in line with what it still
    /// has, since each records something of a graph the schema no longer
    /// says. A `@join__type` for a graph none of the type's fields (or union's
    /// members) are in anymore goes, unless it is the last, since a type with
    /// none is read as being in every graph. So does a `@join__implements` or
    /// `@join__unionMember` naming an interface or member `td` no longer has,
    /// or a graph it is no longer in.
    pub fn drop_dangling_relations(&self, td: &mut TypeDefinition<'_, String>, graphs: &[Graph]) {
        let type_join = self.join("type");
        let implements = self.join("implements");
        let member = self.join("unionMember");
        let had_members = !self.joins(type_directives(td), "unionMember").is_empty();

        // The graphs the type has nothing left in, but for the last of them.
        let type_graphs = self.type_graphs(td, graphs);
        let has_type_joins = !self.joins(type_directives(td), "type").is_empty();
        let mut dropped: Vec<String> = type_graphs
            .iter()
            .filter(|_| has_type_joins)
            .filter(|graph| match &*td {
                TypeDefinition::Object(ObjectType { fields, .. })
                | TypeDefinition::Interface(InterfaceType { fields, .. }) => !fields
                    .iter()
                    .any(|f| self.field_joins(&f.directives, true, graph).is_some()),
                TypeDefinition::Union(union) => {
                    had_members
                        && !self
                            .joins(&union.directives, "unionMember")
                            .iter()
                            .any(|d| {
                                enum_argument(d, "graph") == Some(graph.as_str())
                                    && string_argument(d, "member")
                                        .is_some_and(|m| union.types.iter().any(|kept| kept == m))
                            })
                }
                TypeDefinition::Scalar(_)
                | TypeDefinition::Enum(_)
                | TypeDefinition::InputObject(_) => false,
            })
            .cloned()
            .collect();
        if dropped.len() == type_graphs.len() {
            dropped.pop();
        }

        let (directives, relation, argument, kept): (_, _, _, &[String]) = match td {
            TypeDefinition::Object(ObjectType {
                directives,
                implements_interfaces,
                ..
            })
            | TypeDefinition::Interface(InterfaceType {
                directives,
                implements_interfaces,
                ..
            }) => (directives, &implements, "interface", implements_interfaces),
            TypeDefinition::Union(UnionType {
                directives, types, ..
            }) => (directives, &member, "member", types),
            TypeDefinition::Scalar(_)
            | TypeDefinition::Enum(_)
            | TypeDefinition::InputObject(_) => return,
        };
        let in_dropped = |d: &Directive<'_, String>| {
            enum_argument(d, "graph").is_some_and(|g| dropped.iter().any(|e| e == g))
        };
        directives.retain(|d| {
            if d.name == type_join {
                return !in_dropped(d);
            }
            d.name != *relation
                || (!in_dropped(d)
                    && string_argument(d, argument)
                        .is_some_and(|name| kept.iter().any(|k| k == name)))
        });
    }

    /// Warnings for what the supergraph records that no command follows, so a
    /// gap in the output is never silent: fields whose `@join__field` passes
    /// context arguments (join v0.5's `@fromContext`), whose selections name
    /// fields of other types.
    pub fn untracked(&self, types: &HashMap<String, TypeDefinition<'_, String>>) -> Vec<String> {
        let mut contextual: BTreeSet<String> = BTreeSet::new();
        for (name, td) in types {
            let Some(fields) = util::type_fields(td) else {
                continue;
            };
            for field in fields {
                if self
                    .joins(&field.directives, "field")
                    .iter()
                    .any(|d| argument(d, "contextArguments").is_some())
                {
                    contextual.insert(format!("`{name}.{}`", field.name));
                }
            }
        }
        if contextual.is_empty() {
            return Vec::new();
        }
        let fields: Vec<String> = contextual.into_iter().collect();
        vec![format!(
            "{} {} context arguments (`@join__field(contextArguments:)`), which are not followed, so fields they select may be missing from the output",
            fields.join(", "),
            if fields.len() == 1 { "takes" } else { "take" },
        )]
    }

    /// The schema clients see: every machinery definition and application
    /// removed, and every element marked `@inaccessible` hidden along with
    /// whatever names a hidden type.
    pub fn api_schema<'a>(&self, doc: &Document<'a, String>) -> Document<'a, String> {
        let inaccessible = self
            .features
            .iter()
            .find(|f| f.name == "inaccessible")
            .map(|f| f.directive("inaccessible"));
        let is_hidden = |directives: &[Directive<'_, String>]| {
            inaccessible
                .as_ref()
                .is_some_and(|name| directives.iter().any(|d| d.name == *name))
        };

        let mut hidden: HashSet<String> = HashSet::new();
        for def in &doc.definitions {
            let td = match def {
                Definition::TypeDefinition(td) => td.clone(),
                Definition::TypeExtension(te) => util::extension_as_definition(te),
                Definition::SchemaDefinition(_) | Definition::DirectiveDefinition(_) => continue,
            };
            let name = util::type_definition_name(&td);
            if self.is_machinery_type(name) || is_hidden(type_directives(&td)) {
                hidden.insert(name.clone());
            }
        }

        let api = Api {
            supergraph: self,
            hidden: &hidden,
            is_hidden: &is_hidden,
        };
        let definitions = doc
            .definitions
            .iter()
            .filter_map(|def| match def {
                Definition::SchemaDefinition(sd) => {
                    let mut sd = sd.clone();
                    sd.directives = api.directives(&sd.directives);
                    Some(Definition::SchemaDefinition(sd))
                }
                Definition::DirectiveDefinition(dd) => {
                    (!self.is_machinery_directive(&dd.name)).then(|| def.clone())
                }
                Definition::TypeDefinition(td) => {
                    let name = util::type_definition_name(td);
                    (!hidden.contains(name)).then(|| Definition::TypeDefinition(api.type_(td)))
                }
                Definition::TypeExtension(te) => {
                    let td = util::extension_as_definition(te);
                    if hidden.contains(util::type_definition_name(&td)) {
                        return None;
                    }
                    util::definition_as_extension(api.type_(&td)).map(Definition::TypeExtension)
                }
            })
            .collect();
        Document { definitions }
    }
}

/// What `Supergraph::api_schema` removes from each definition it keeps.
struct Api<'s, F: Fn(&[Directive<'_, String>]) -> bool> {
    supergraph: &'s Supergraph,
    /// Machinery types and types marked `@inaccessible`.
    hidden: &'s HashSet<String>,
    /// Whether directives mark their element `@inaccessible`.
    is_hidden: &'s F,
}

impl<F: Fn(&[Directive<'_, String>]) -> bool> Api<'_, F> {
    fn directives<'a>(&self, directives: &[Directive<'a, String>]) -> Vec<Directive<'a, String>> {
        directives
            .iter()
            .filter(|d| !self.supergraph.is_machinery_directive(&d.name))
            .cloned()
            .collect()
    }

    fn names_hidden(&self, ty: &Type<'_, String>) -> bool {
        util::named_type(ty).is_some_and(|name| self.hidden.contains(name))
    }

    fn names(&self, names: &[String]) -> Vec<String> {
        names
            .iter()
            .filter(|name| !self.hidden.contains(*name))
            .cloned()
            .collect()
    }

    fn fields<'a>(&self, fields: &[Field<'a, String>]) -> Vec<Field<'a, String>> {
        fields
            .iter()
            .filter(|f| !(self.is_hidden)(&f.directives) && !self.names_hidden(&f.field_type))
            .map(|f| Field {
                arguments: self.input_values(&f.arguments),
                directives: self.directives(&f.directives),
                ..f.clone()
            })
            .collect()
    }

    fn input_values<'a>(&self, values: &[InputValue<'a, String>]) -> Vec<InputValue<'a, String>> {
        values
            .iter()
            .filter(|v| !(self.is_hidden)(&v.directives) && !self.names_hidden(&v.value_type))
            .map(|v| InputValue {
                directives: self.directives(&v.directives),
                ..v.clone()
            })
            .collect()
    }

    fn type_<'a>(&self, td: &TypeDefinition<'a, String>) -> TypeDefinition<'a, String> {
        match td {
            TypeDefinition::Scalar(scalar) => TypeDefinition::Scalar(ScalarType {
                directives: self.directives(&scalar.directives),
                ..scalar.clone()
            }),
            TypeDefinition::Object(obj) => TypeDefinition::Object(ObjectType {
                implements_interfaces: self.names(&obj.implements_interfaces),
                directives: self.directives(&obj.directives),
                fields: self.fields(&obj.fields),
                ..obj.clone()
            }),
            TypeDefinition::Interface(iface) => TypeDefinition::Interface(InterfaceType {
                implements_interfaces: self.names(&iface.implements_interfaces),
                directives: self.directives(&iface.directives),
                fields: self.fields(&iface.fields),
                ..iface.clone()
            }),
            TypeDefinition::Union(union) => TypeDefinition::Union(UnionType {
                directives: self.directives(&union.directives),
                types: self.names(&union.types),
                ..union.clone()
            }),
            TypeDefinition::Enum(enum_type) => TypeDefinition::Enum(EnumType {
                directives: self.directives(&enum_type.directives),
                values: enum_type
                    .values
                    .iter()
                    .filter(|v| !(self.is_hidden)(&v.directives))
                    .map(|v| graphql_parser::schema::EnumValue {
                        directives: self.directives(&v.directives),
                        ..v.clone()
                    })
                    .collect(),
                ..enum_type.clone()
            }),
            TypeDefinition::InputObject(input) => TypeDefinition::InputObject(InputObjectType {
                directives: self.directives(&input.directives),
                fields: self.input_values(&input.fields),
                ..input.clone()
            }),
        }
    }
}

/// The directives applied to a type definition itself.
pub fn type_directives<'r, 'a>(td: &'r TypeDefinition<'a, String>) -> &'r [Directive<'a, String>] {
    match td {
        TypeDefinition::Scalar(t) => &t.directives,
        TypeDefinition::Object(t) => &t.directives,
        TypeDefinition::Interface(t) => &t.directives,
        TypeDefinition::Union(t) => &t.directives,
        TypeDefinition::Enum(t) => &t.directives,
        TypeDefinition::InputObject(t) => &t.directives,
    }
}

/// The value of a directive's argument `name`.
pub fn argument<'d, 'a>(
    directive: &'d Directive<'a, String>,
    name: &str,
) -> Option<&'d Value<'a, String>> {
    directive
        .arguments
        .iter()
        .find(|(argument, _)| argument == name)
        .map(|(_, value)| value)
}

/// A directive's argument `name`, when it is a string.
pub fn string_argument<'d>(directive: &'d Directive<'_, String>, name: &str) -> Option<&'d str> {
    match argument(directive, name) {
        Some(Value::String(value)) => Some(value),
        _ => None,
    }
}

/// A directive's argument `name`, when it is an enum value.
pub fn enum_argument<'d>(directive: &'d Directive<'_, String>, name: &str) -> Option<&'d str> {
    match argument(directive, name) {
        Some(Value::Enum(value)) => Some(value),
        _ => None,
    }
}

/// Whether a directive's argument `name` is `true`. Absent is `false`, the
/// default of every boolean join argument but `resolvable`.
pub fn is_true(directive: &Directive<'_, String>, name: &str) -> bool {
    matches!(argument(directive, name), Some(Value::Boolean(true)))
}

/// Parses a field set, the selection a join directive names in a string
/// (`"id org { id }"`), by reading it as the body of a query, so nested
/// selections and inline fragments parse as they would there.
pub fn parse_field_set(text: &str) -> Option<SelectionSet<'static, String>> {
    let text = format!("{{{text}}}");
    let document = query::parse_query::<String>(&text).ok()?;
    let mut definitions = document.into_static().definitions.into_iter();
    match (definitions.next(), definitions.next()) {
        (
            Some(query::Definition::Operation(query::OperationDefinition::SelectionSet(set))),
            None,
        ) => Some(set),
        _ => None,
    }
}

/// Parses a type reference such as `[Review!]!`, as `@join__field(type:)`
/// writes one, by reading it as a field's type.
pub fn parse_type<'a>(text: &str) -> Option<Type<'a, String>> {
    let text = format!("type T {{ f: {text} }}");
    let document = graphql_parser::parse_schema::<String>(&text).ok()?;
    match document.definitions.into_iter().next()? {
        Definition::TypeDefinition(TypeDefinition::Object(mut obj)) if obj.fields.len() == 1 => {
            Some(owned_type(obj.fields.remove(0).field_type))
        }
        _ => None,
    }
}

/// A type reference unbound from the text it was parsed from, which holds no
/// borrow of it, since its names are `String`s.
fn owned_type<'a>(ty: Type<'_, String>) -> Type<'a, String> {
    match ty {
        Type::NamedType(name) => Type::NamedType(name),
        Type::ListType(inner) => Type::ListType(Box::new(owned_type(*inner))),
        Type::NonNullType(inner) => Type::NonNullType(Box::new(owned_type(*inner))),
    }
}

fn not_a_selection(what: &str, text: &str, owner: &str) -> String {
    format!("the {what} `{text}` of `{owner}` is not a selection of fields")
}

/// `Supergraph::field_set_fields`, collecting into `out`.
fn walk_field_set(
    sg: &Supergraph,
    types: &HashMap<String, TypeDefinition<'_, String>>,
    on: &str,
    selection: &SelectionSet<'_, String>,
    graph: Option<&str>,
    out: &mut HashSet<(String, String)>,
) {
    for item in &selection.items {
        match item {
            Selection::Field(field) => {
                out.insert((on.to_string(), field.name.clone()));
                if field.selection_set.items.is_empty() {
                    continue;
                }
                let Some(definition) = types
                    .get(on)
                    .and_then(util::type_fields)
                    .and_then(|fields| fields.iter().find(|f| f.name == field.name))
                else {
                    continue;
                };
                let ty = match graph {
                    Some(g) => sg.type_in(definition, g),
                    None => definition.field_type.clone(),
                };
                if let Some(returns) = util::named_type(&ty) {
                    walk_field_set(sg, types, returns, &field.selection_set, graph, out);
                }
            }
            Selection::InlineFragment(fragment) => {
                let on = util::type_condition(fragment.type_condition.as_ref()).unwrap_or(on);
                walk_field_set(sg, types, on, &fragment.selection_set, graph, out);
            }
            Selection::FragmentSpread(_) => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_field_set, Supergraph};
    use crate::util;
    use graphql_parser::parse_schema;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    const SUPERGRAPH: &str = include_str!("../tests/fixtures/supergraph.graphql");

    fn detect(schema: &str) -> Option<Supergraph> {
        Supergraph::detect(&parse_schema::<String>(schema).unwrap())
    }

    /// The smallest supergraph: `@link` linking itself and join, one graph.
    fn minimal(link: &str, body: &str) -> String {
        format!(
            "schema @link(url: \"https://specs.apollo.dev/link/v1.0\") {link} {{ query: Query }}\n\
             enum join__Graph {{ A @join__graph(name: \"a\", url: \"http://a\") }}\n{body}"
        )
    }

    #[test]
    fn a_supergraph_is_a_schema_linking_the_join_spec() {
        assert!(detect(SUPERGRAPH).is_some());
        assert!(detect("type Query { a: Int }").is_none());
        // `@link` alone makes a subgraph or any linked schema, not a supergraph.
        let linked = indoc! {r#"
            schema @link(url: "https://specs.apollo.dev/link/v1.0") @link(url: "https://specs.apollo.dev/federation/v2.3") { query: Query }
            type Query { a: Int }
        "#};
        assert!(detect(linked).is_none());
        // Nor does a directive named like a join directive make one.
        assert!(detect("type Query @join__type(graph: A) { a: Int }").is_none());
    }

    #[test]
    fn machinery_is_what_the_linked_specs_define() {
        let sg = detect(SUPERGRAPH).unwrap();
        for name in [
            "join__Graph",
            "join__FieldSet",
            "link__Import",
            "link__Purpose",
        ] {
            assert!(sg.is_machinery_type(name), "{name}");
        }
        for name in ["Query", "User", "Link", "joinGraph"] {
            assert!(!sg.is_machinery_type(name), "{name}");
        }
        for name in ["link", "join__type", "join__field", "inaccessible"] {
            assert!(sg.is_machinery_directive(name), "{name}");
        }
        for name in ["deprecated", "key", "tag"] {
            assert!(!sg.is_machinery_directive(name), "{name}");
        }
        assert!(!sg.is_federation_1());
    }

    /// A spec linked `as:` another name has its elements under that prefix,
    /// and an imported element under the name it is imported as.
    #[test]
    fn renamed_and_imported_elements_are_machinery_under_their_new_names() {
        let schema = minimal(
            r#"@link(url: "https://specs.apollo.dev/join/v0.3", as: "j") @link(url: "https://specs.apollo.dev/tag/v0.3", import: [{name: "@tag", as: "@label"}])"#,
            "type Query @j__type(graph: A) { a: Int @label(name: \"x\") }",
        );
        let schema = schema
            .replace("join__Graph", "j__Graph")
            .replace("join__graph", "j__graph");
        let sg = detect(&schema).unwrap();
        assert!(sg.is_machinery_type("j__Graph"));
        assert!(!sg.is_machinery_type("join__Graph"));
        assert!(sg.is_machinery_directive("j__type"));
        assert!(sg.is_machinery_directive("label"));
        assert_eq!(sg.carried("label"), Some("tag"));
        assert_eq!(sg.join("field"), "j__field");
    }

    #[test]
    fn federation_1_links_with_core() {
        let schema = indoc! {r#"
            schema @core(feature: "https://specs.apollo.dev/core/v0.2") @core(feature: "https://specs.apollo.dev/join/v0.1", for: EXECUTION) { query: Query }
            type Query { a: Int }
        "#};
        assert!(detect(schema).unwrap().is_federation_1());
    }

    #[test]
    fn field_sets_are_read_from_keys_requires_and_provides() {
        let doc = parse_schema::<String>(SUPERGRAPH).unwrap();
        let sg = Supergraph::detect(&doc).unwrap();
        let types = util::merged_type_definitions(&doc);
        let mut sets: Vec<String> = sg
            .field_sets(&types)
            .unwrap()
            .iter()
            .map(|fs| {
                let mut named: Vec<String> = sg
                    .field_set_fields(&types, fs, None)
                    .into_iter()
                    .map(|(t, f)| format!("{t}.{f}"))
                    .collect();
                named.sort();
                let owner = match &fs.field {
                    Some(field) => format!("{}.{field}", fs.type_name),
                    None => fs.type_name.clone(),
                };
                format!("{owner} on {}: {}", fs.on, named.join(" "))
            })
            .collect();
        sets.sort();
        sets.dedup();
        assert_eq!(
            sets,
            [
                "Book on Book: Book.id",
                "Media on Media: Media.id",
                "Org on Org: Org.id",
                "Product on Product: Product.upc",
                "Product.shipping on Product: Product.weight",
                "Review.author on User: User.username",
                "Team on Team: Org.id Team.org Team.slug",
                "User on User: User.id",
            ]
        );
    }

    #[test]
    fn a_field_set_that_is_not_a_selection_is_an_error() {
        let schema = minimal(
            r#"@link(url: "https://specs.apollo.dev/join/v0.4")"#,
            r#"type Query @join__type(graph: A, key: "id {") { id: ID }"#,
        );
        let doc = parse_schema::<String>(&schema).unwrap();
        let sg = Supergraph::detect(&doc).unwrap();
        let err = sg
            .field_sets(&util::merged_type_definitions(&doc))
            .unwrap_err();
        assert!(err.contains("`id {`"), "{err}");
        assert!(parse_field_set("id org { id } ... on A { b }").is_some());
        assert!(parse_field_set("}").is_none());
    }

    /// What clients see: no machinery, and nothing `@inaccessible`.
    #[test]
    fn the_api_schema_hides_machinery_and_inaccessible_elements() {
        let doc = parse_schema::<String>(SUPERGRAPH).unwrap();
        let api = detect(SUPERGRAPH).unwrap().api_schema(&doc).to_string();
        for hidden in ["join__", "link__", "@link", "@inaccessible", "secret"] {
            assert!(!api.contains(hidden), "{hidden} in:\n{api}");
        }
        util::assert_self_contained(&api);
        assert!(api.contains("type User implements Node {\n  id: ID!\n  username: String\n  balance: Money\n  reviews: [Review]\n}"), "{api}");
    }

    /// An inaccessible type takes every field, argument, and member naming it.
    #[test]
    fn the_api_schema_drops_what_names_an_inaccessible_type() {
        let schema = minimal(
            r#"@link(url: "https://specs.apollo.dev/join/v0.4") @link(url: "https://specs.apollo.dev/inaccessible/v0.2", for: SECURITY)"#,
            indoc! {"
                directive @inaccessible on OBJECT | FIELD_DEFINITION | ENUM_VALUE | ARGUMENT_DEFINITION
                type Query { a(x: Int @inaccessible, y: Int): Int, secret: Secret, any: Any }
                type Secret @inaccessible { a: Int }
                type Open { a: Int }
                union Any = Secret | Open
                enum E { A, B @inaccessible }
            "},
        );
        let doc = parse_schema::<String>(&schema).unwrap();
        let api = detect(&schema).unwrap().api_schema(&doc).to_string();
        assert_eq!(
            api,
            indoc! {"
                schema {
                  query: Query
                }

                type Query {
                  a(y: Int): Int
                  any: Any
                }

                type Open {
                  a: Int
                }

                union Any = Open

                enum E {
                  A
                }
            "}
        );
    }

    /// What a supergraph records that no command follows is a warning, so a
    /// field missing from the output for it is never a silent gap.
    #[test]
    fn context_arguments_are_not_followed_and_say_so() {
        let schema = minimal(
            r#"@link(url: "https://specs.apollo.dev/join/v0.5", for: EXECUTION)"#,
            indoc! {r#"
                type Query @join__type(graph: A) {
                  a(x: Int): Int @join__field(graph: A, contextArguments: [{context: "c", name: "x", type: "Int", selection: "{ id }"}])
                  b: Int
                }
            "#},
        );
        let doc = parse_schema::<String>(&schema).unwrap();
        let sg = Supergraph::detect(&doc).unwrap();
        assert_eq!(
            sg.untracked(&util::merged_type_definitions(&doc)),
            ["`Query.a` takes context arguments (`@join__field(contextArguments:)`), which are not followed, so fields they select may be missing from the output"]
        );
        let doc = parse_schema::<String>(SUPERGRAPH).unwrap();
        let sg = Supergraph::detect(&doc).unwrap();
        assert_eq!(
            sg.untracked(&util::merged_type_definitions(&doc)),
            Vec::<String>::new()
        );
    }
}
