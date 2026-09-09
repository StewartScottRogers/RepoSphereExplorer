-- A small service configuration, exercising every field the `dhall`
-- plugin extracts: a hashed import, a union type, a record type, a
-- lambda with a type annotation, and a `merge` over the union.
let Prelude = https://prelude.dhall-lang.org/v21.1.0/package.dhall sha256:6b90326dc39ab738d7ed87b970ba675c496bed0194071b332840a87261649dc

let Environment = < Development | Staging | Production : Text >

let ServiceConfig = { name : Text, port : Natural, environment : Environment }

let describeEnvironment : Environment -> Text = \(environment : Environment) -> merge { Development = "development", Staging = "staging", Production = \(region : Text) -> "production (${region})" } environment

let service : ServiceConfig = { name = "repos-explorer-api", port = 8080, environment = Environment.Production "us-east-1" }

in  { service = service, summary = describeEnvironment service.environment }
