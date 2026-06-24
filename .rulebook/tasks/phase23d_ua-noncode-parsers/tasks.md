## 1. Parser registry
- [x] 1.1 Define `Parser` trait (`matches(path)`, `parse(content, path) -> facts`)
- [x] 1.2 Registry: first-match-wins; code paths fall back to the existing extractor
- [x] 1.3 Parser facts flow through the phase23c reconciliation gate before upsert

## 2. SQL parser
- [x] 2.1 Emit `table`/`schema` nodes + `defines_schema`/`migrates`/`reads_from`/`writes_to` edges
- [x] 2.2 Golden-file test on a fixture `.sql`

## 3. Terraform parser
- [x] 3.1 Emit `resource`/`service` nodes + `provisions`/`depends_on` edges
- [x] 3.2 Golden-file test on a fixture `.tf`

## 4. protobuf parser
- [x] 4.1 Emit `schema`/`endpoint`/`service` nodes + `defines_schema`/`routes` edges
- [x] 4.2 Golden-file test on a fixture `.proto`

## 5. GraphQL parser
- [x] 5.1 Emit `schema`/`endpoint` nodes + `defines_schema`/`routes` edges
- [x] 5.2 Golden-file test on a fixture `.graphql`

## 6. Dockerfile parser
- [x] 6.1 Emit `config`/`service` nodes + `deploys`/`depends_on` edges
- [x] 6.2 Golden-file test on a fixture `Dockerfile`

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation
- [x] 7.2 Write tests covering the new behavior (registry dispatch + per-parser golden files)
- [x] 7.3 Run tests and confirm they pass
