## 1. Parser registry
- [ ] 1.1 Define `Parser` trait (`matches(path)`, `parse(content, path) -> facts`)
- [ ] 1.2 Registry: first-match-wins; code paths fall back to the existing extractor
- [ ] 1.3 Parser facts flow through the phase23c reconciliation gate before upsert

## 2. SQL parser
- [ ] 2.1 Emit `table`/`schema` nodes + `defines_schema`/`migrates`/`reads_from`/`writes_to` edges
- [ ] 2.2 Golden-file test on a fixture `.sql`

## 3. Terraform parser
- [ ] 3.1 Emit `resource`/`service` nodes + `provisions`/`depends_on` edges
- [ ] 3.2 Golden-file test on a fixture `.tf`

## 4. protobuf parser
- [ ] 4.1 Emit `schema`/`endpoint`/`service` nodes + `defines_schema`/`routes` edges
- [ ] 4.2 Golden-file test on a fixture `.proto`

## 5. GraphQL parser
- [ ] 5.1 Emit `schema`/`endpoint` nodes + `defines_schema`/`routes` edges
- [ ] 5.2 Golden-file test on a fixture `.graphql`

## 6. Dockerfile parser
- [ ] 6.1 Emit `config`/`service` nodes + `deploys`/`depends_on` edges
- [ ] 6.2 Golden-file test on a fixture `Dockerfile`

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
- [ ] 7.2 Write tests covering the new behavior (registry dispatch + per-parser golden files)
- [ ] 7.3 Run tests and confirm they pass
