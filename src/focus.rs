use graphql_parser::parse_schema;
use graphql_parser::schema::{
    Definition, Document, Field, InputObjectType, InputValue, InterfaceType, ObjectType, Type,
    TypeDefinition,
};
use std::collections::{HashSet, VecDeque};

use crate::util::{self, SchemaWalker};

pub fn process(schema: &str, r#type: &str) -> String {
    let schema_ast = parse_schema::<String>(schema).expect("Invalid schema");

    // let used = find_used_types(&schema_ast, r#type);
    let mut utc = UsedTypeCollector::new(r#type.to_string());
    utc.walk_schema(&schema_ast);
    let UsedTypeCollector { used, .. } = utc;

    strip_unused_types(&schema_ast, used)
    // strip_unused_types(&schema_ast, used.iter().collect())
}

#[derive(Debug)]
struct UsedTypeCollector<'a> {
    used: HashSet<&'a String>,
    root_type: String,
}

impl UsedTypeCollector<'_> {
    fn new(root_type: String) -> Self {
        Self {
            used: HashSet::new(),
            root_type,
        }
    }
}

impl<'a> util::SchemaWalker<'a, String, String, HashSet<&'a String>> for UsedTypeCollector<'a> {
    fn select_object_fields(
        &mut self,
        obj: &'a ObjectType<'a, String>,
        path: &[&'a String],
    ) -> Vec<&'a Field<'a, String>> {
        if obj.name == self.root_type || path.contains(&&self.root_type) {
            return obj.fields.iter().collect();
        }

        obj.fields
            .iter()
            .filter(|f| self.root_type == f.name)
            .collect()
    }

    fn select_interface_fields(
        &mut self,
        iface: &'a InterfaceType<'a, String>,
        path: &[&'a String],
    ) -> Vec<&'a Field<'a, String>> {
        if iface.name == self.root_type || path.contains(&&self.root_type) {
            return iface.fields.iter().collect();
        }

        iface
            .fields
            .iter()
            .filter(|f| self.root_type == f.name)
            .collect()
    }

    fn select_input_fields(
        &mut self,
        input: &'a InputObjectType<'a, String>,
        path: &[&'a String],
    ) -> Vec<&'a InputValue<'a, String>> {
        if input.name == self.root_type || path.contains(&&self.root_type) {
            return input.fields.iter().collect();
        }

        input
            .fields
            .iter()
            .filter(|f| self.root_type == f.name)
            .collect()
    }

    fn visit_field(&mut self, field: &'a Field<'a, String>, path: &[&'a String]) {
        if field.name == self.root_type || path.contains(&&self.root_type) {
            self.used
                .insert(util::named_type(&field.field_type).unwrap());
        }
    }

    fn visit_type_definition(&mut self, ty: &'a TypeDefinition<'a, String>, path: &[&'a String]) {
        let type_name = util::schema_type_definition_name(ty).unwrap();
        if type_name == &self.root_type || path.contains(&&self.root_type) {
            self.used.insert(type_name);
        }
    }
}

// Identifies and returns a set of used type names in the GraphQL schema.
// It starts from the specified root type and operation name, then recursively explores
// all related types to determine which types are utilized.
fn find_used_types<'a>(schema: &'a Document<'a, String>, r#type: &str) -> HashSet<String> {
    let mut used_types = HashSet::new();
    let mut queue = VecDeque::new();

    queue.push_back(r#type.to_string());

    while let Some(type_name) = queue.pop_front() {
        if !used_types.insert(type_name.clone()) {
            continue;
        }

        for def in schema.definitions.iter() {
            if util::schema_definition_name(def) != Some(&type_name) {
                continue;
            }
            match def {
                Definition::SchemaDefinition(_) => (),
                Definition::TypeDefinition(td) => match td {
                    TypeDefinition::Scalar(_) => (),
                    TypeDefinition::Object(object_type) => {
                        for f in &object_type.fields {
                            queue.push_back(get_base_type(&f.field_type));
                        }
                    }
                    TypeDefinition::Interface(interface_type) => {
                        for f in &interface_type.fields {
                            queue.push_back(get_base_type(&f.field_type));
                        }

                        for d in schema.definitions.iter() {
                            if let Definition::TypeDefinition(TypeDefinition::Object(o)) = d {
                                if o.implements_interfaces
                                    .iter()
                                    .any(|i| i == &interface_type.name)
                                {
                                    queue.push_back(o.name.clone());
                                }
                            }
                        }
                    }
                    TypeDefinition::Union(union_type) => {
                        for t in &union_type.types {
                            queue.push_back(t.clone());
                        }
                    }
                    TypeDefinition::Enum(_) => (),
                    TypeDefinition::InputObject(input_object_type) => {
                        for f in &input_object_type.fields {
                            queue.push_back(get_base_type(&f.value_type));
                        }
                    }
                },
                Definition::TypeExtension(_) => (),
                Definition::DirectiveDefinition(_) => (),
            }
        }
    }

    used_types
}

/// Extracts the base type name from a GraphQL Type.
/// It handles NamedType, ListType, and NonNullType by recursively resolving to the underlying named type.
fn get_base_type(t: &Type<String>) -> String {
    match t {
        graphql_parser::schema::Type::NamedType(name) => name.clone(),
        graphql_parser::schema::Type::ListType(inner) => get_base_type(inner),
        graphql_parser::schema::Type::NonNullType(inner) => get_base_type(inner),
    }
}

/// Removes unused types from the GraphQL schema.
/// It filters out definitions that are not in the set of used types and returns the modified schema as a string.
fn strip_unused_types<'a>(
    schema: &'a Document<'a, String>,
    used_types: HashSet<&String>,
) -> String {
    let retained: Vec<_> = schema
        .definitions
        .iter()
        .filter(|def| {
            util::schema_definition_name(def)
                .map_or_else(|| false, |name| used_types.contains(name))
        })
        .collect();

    let result_doc = Document {
        definitions: retained.into_iter().cloned().collect(),
    };

    format!("{}", result_doc)
}

#[cfg(test)]
mod tests {
    use crate::focus;
    use indoc::indoc;

    #[test]
    fn test_focus_query_operation() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            type User {
              id: ID
              name: String
            }
        "};

        let result = focus::process(schema, "User");
        let expected_schema = indoc! {"
            type User {
              id: ID
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_interface() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            interface Person {
              name: String
            }

            type User implements Person {
              id: ID
              name: String
              admin: Bool
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        let result = focus::process(schema, "Person");
        let expected_schema = indoc! {"
            interface Person {
              name: String
            }

            type User implements Person {
              id: ID
              name: String
              admin: Bool
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_nested_interface() {
        let schema = indoc! {"
            type Query {
                company: Company
            }

            type Company {
              employees: [Person]
            }

            interface Person {
              name: String
            }

            type User implements Person {
              id: ID
              name: String
              admin: Bool
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        let result = focus::process(schema, "Company");
        let expected_schema = indoc! {"
            type Company {
              employees: [Person]
            }

            interface Person {
              name: String
            }

            type User implements Person {
              id: ID
              name: String
              admin: Bool
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_query_missing_operation() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            type User {
              id: ID
              name: String
            }
        "};

        let result = focus::process(schema, "nonExistent");
        assert_eq!(result.trim(), "");
    }

    #[test]
    fn test_focus_nested_types() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            type User {
              id: ID
              profile: Profile
            }

            type Profile {
              email: String
            }
        "};

        let result = focus::process(schema, "User");
        let expected_schema = indoc! {"
            type User {
              id: ID
              profile: Profile
            }

            type Profile {
              email: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_unused_types() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            type User {
              id: ID
              profile: Profile
            }

            type Profile {
              email: String
            }

            type UnusedType {
              field: String
            }
        "};

        let result = focus::process(schema, "User");
        let expected_schema = indoc! {"
            type User {
              id: ID
              profile: Profile
            }

            type Profile {
              email: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }
}
