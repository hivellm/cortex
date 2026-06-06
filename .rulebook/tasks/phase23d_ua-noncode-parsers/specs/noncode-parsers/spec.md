# Non-Code Parsers

## ADDED Requirements

### Requirement: Pluggable parser registry
The system SHALL provide a deterministic parser registry that dispatches a file to the
first parser whose matcher accepts it (by extension or filename), falling back to the
existing code extractor, and every parser's output MUST pass through the extraction
reconciliation gate before graph upsert.

#### Scenario: Dispatch by extension
Given a registry containing a SQL parser
When a file ending in `.sql` is indexed
Then the SQL parser handles it and produces graph facts

#### Scenario: Code falls back to extractor
Given a registry of non-code parsers
When a `.rs` source file is indexed
Then no non-code parser matches and the existing code extractor handles it

### Requirement: Infra and data nodes
The system SHALL emit the adopted node and edge kinds from the priority parsers — SQL
(`table`/`schema`, `defines_schema`/`migrates`), Terraform (`resource`, `provisions`),
protobuf and GraphQL (`schema`/`endpoint`, `defines_schema`/`routes`), and Dockerfile
(`config`/`service`, `deploys`).

#### Scenario: SQL DDL yields a table node
Given a `.sql` file with a `CREATE TABLE users` statement
When the SQL parser runs
Then a `table` node for `users` is emitted with a `defines_schema` edge

#### Scenario: Terraform resource is provisioned
Given a `.tf` file declaring an `aws_s3_bucket` resource
When the Terraform parser runs
Then a `resource` node is emitted with a `provisions` edge
