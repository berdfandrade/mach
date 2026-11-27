# MACH

MACH é um framework de backend em Rust para criar APIs com PostgreSQL. Você declara uma entidade em Rust, escolhe quais campos a API pode receber e devolver, e usa a CLI para gerar o controlador, inspecionar o SQL e preparar migrações. Axum atende às requisições HTTP e SQLx executa as consultas parametrizadas.

A mesma CLI serve a pessoas e agentes de programação. Por exemplo, `mach field rename User name full_name --dry-run` mostra quais declarações e consultas serão afetadas antes de editar o código. A geração é determinística e local, sem dependência de um modelo de IA.

## Experimente

Com Rust estável, rustfmt e Clippy instalados, execute na raiz deste repositório. O projeto foi verificado com Rust 1.95.

```sh
cargo run -p hello-world
```

Em outro terminal, `curl http://127.0.0.1:3000/` retorna `Hello, MACH!`. Esse exemplo funciona sem banco de dados. Encerre o processo com Ctrl+C antes de iniciar outra aplicação na mesma porta.

## Crie uma API de usuários

```sh
cargo install --path crates/mach-cli --locked
mach new demo --db postgres
cd demo
mach generate resource users --crud
mach field add User phone:string --optional
mach field rename User name full_name --dry-run
mach field rename User name full_name
mach generate resource posts --crud --owner users
mach field add Post created_at:timestamp --generated --expose
mach generate migration
mach inspect User --sql
mach context --compact
mach check --agent
mach test
mach build --release
```

O projeto gerado referencia o crate `mach` deste checkout. Para usar outra cópia, defina `MACH_FRAMEWORK_PATH` antes de executar `mach new`. Versione o `Cargo.lock` gerado e ajuste a dependência local se mover a aplicação para outra máquina.

Para executar a API com um PostgreSQL local, revise o SQL em `migrations/` e rode os comandos abaixo no diretório `demo`. Eles criam credenciais de desenvolvimento e liberam a criação de usuários e a leitura do usuário de ID 1.

```sh
export POSTGRES_PASSWORD="$(openssl rand -hex 32)"
export DATABASE_URL="postgres://app:${POSTGRES_PASSWORD}@127.0.0.1:5432/app"
export MACH_TOKEN="$(openssl rand -hex 32)"
export MACH_GRANTS="users:create:collection,users:read:1"
docker compose up -d --wait
mach migrate
mach dev
```

Em outro terminal com o mesmo `MACH_TOKEN`, crie o primeiro usuário.

```sh
curl -i http://127.0.0.1:3000/users \
  -H "Authorization: Bearer ${MACH_TOKEN}" \
  -H 'Content-Type: application/json' \
  -d '{"email":"ana@example.org","full_name":"Ana"}'
```

Em um banco novo, a resposta é `201 Created` e inclui o ID gerado. O campo `phone` fica privado até receber uma declaração explícita de visibilidade. A listagem exige sua própria permissão. Requisições sem credencial recebem 401 e operações sem permissão recebem 403. A API e a porta do banco escutam em localhost por padrão. Em um ambiente publicado, configure TLS na entrada do serviço.

## Como o framework funciona

- **Entidades e validação.** `#[entity(table = "users")]` declara a tabela. `#[id] #[generated]` identifica a chave primária i64. `#[writable]` inclui um campo na entrada pública e `#[expose]` permite serializá-lo. A entrada rejeita campos desconhecidos. O modelo aceita String, i32, i64, bool, `Timestamp` (TIMESTAMPTZ, serializado em RFC 3339) e Option desses tipos. `#[generated]` em um `Timestamp` deixa o banco preencher o valor com `now()`. PUT substitui os campos graváveis. O recurso `users` gerado já declara o e-mail como único.
- **Consultas.** O repositório deriva métodos com os prefixos `find_by`, `count_by`, `exists_by` e `delete_by`. Predicados aceitam igualdade, `and`, `is_null`, `greater_than` e `less_than`. O compilador verifica nomes de campos, ambiguidades, parâmetros e retorno. Uma busca individual retorna a primeira linha em ordem de chave primária. A listagem retorna 25 registros por padrão, com limite de 100 e offset máximo de um milhão. `?order=desc` inverte a ordem da chave primária.
- **Persistência.** CRUD e consultas recebem o pool ou a transação explicitamente. `#[references("outra_tabela.id")]`, `#[unique]` e `#[index]` declaram chaves estrangeiras e índices de uma coluna. Para SQL específico, use `mach::sqlx` e associe valores externos por parâmetros.
- **Dono do registro.** `#[owner]` marca um campo i64 que o cliente nunca envia. `create`, `update` e `delete` recebem o dono explicitamente, e as escritas filtram por ele: alterar o registro de outro dono resulta em 404. `generate resource posts --crud --owner users` cria o campo `user_id` e um controlador que usa `s.require(...)?.id()?`, o ID numérico do principal autenticado. A política de acesso precisa devolver esse ID; o `BearerAccess` padrão representa um único operador e recusa essas escritas.
- **WebSocket.** Ative a feature `ws` do crate `mach` para usar `mach::axum::extract::ws`.
- **Alterações de código e schema.** A CLI edita declarações Rust que consegue identificar, preserva texto não relacionado e verifica a compilação. Um diário em `.mach/` permite restaurar os arquivos após falhas. Em caso de edição concorrente conflitante, a recuperação exige conciliação manual. As migrações SQL são numeradas e versionadas. Uma chave estável permite renomear colunas preservando os dados. Uma coluna obrigatória sem valor padrão adicionada a uma tabela existente gera um aviso `review`, porque as linhas atuais precisam de preenchimento antes da migração. Conversões de tipo, mudanças de valores padrão existentes e reformulações de relacionamentos exigem SQL revisado e conciliação do histórico.
- **MACH Guard.** Combina diagnósticos do compilador e do Clippy com regras de arquitetura, comentários, código inseguro e operações potencialmente bloqueantes. Os resultados distinguem violações comprovadas de avisos heurísticos. `mach fix --safe` aplica rustfmt com verificação e restauração em caso de falha.

## Comandos de referência

| Comando | Uso |
|---|---|
| `mach generate` | Gera módulos, entidades, controladores, serviços, recursos CRUD ou migrações. |
| `mach field`, `mach index` | Adiciona, renomeia, altera ou remove campos e declara índices. Aceitam `--dry-run`. |
| `mach migrate` | Aplica migrações revisadas. Mudanças destrutivas exigem `--allow-destructive`. |
| `mach context`, `inspect`, `routes`, `dependencies`, `schema`, `api` | Consulta metadados, rotas, SQL e APIs. Use `--json` para automação. |
| `mach expand` | Mostra a expansão das macros usando a ferramenta opcional cargo-expand. |
| `mach check --agent`, `mach check --strict` | Produz diagnósticos estruturados ou aplica a política estrita de comentários. |
| `mach test --affected`, `mach build --release`, `mach dev` | Executa testes, compila ou inicia a aplicação usando Cargo. |

## Exemplos e verificação

O [users-api](examples/users-api) contém a entidade, o controlador, o repositório e a migração do exemplo. O [authenticated-api](examples/authenticated-api) reutiliza as rotas e limita a leitura ao ID associado à credencial. Execute-o a partir do diretório de users-api com o mesmo banco, uma credencial bearer e `MACH_ACCOUNT_ID` definido.

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo test --locked -p mach-cli --test postgres -- --ignored
python3 -m unittest discover -s benchmarks
```

O teste de integração usa Docker, PostgreSQL 16 e Python 3. Ele cria uma aplicação e um banco descartáveis e verifica persistência, autorização por recurso, renomeação de colunas com dados existentes e restauração após falhas de edição.

## Benchmarks

O benchmark HTTP compara MACH e Axum com os mesmos limites, credenciais, dados e configuração de banco. No Linux, com os binários de servidor do PostgreSQL instalados, execute `python3 benchmarks/run_http.py --requests 10000 --concurrency 1 16 --repeats 3`. A [medição local](benchmarks/results/local-2026-11-25.json) reúne 64 mil requisições em oito cargas de trabalho, em uma máquina compartilhada.

O [BackendBench](benchmarks/BackendBench.md) define tarefas para comparar o custo de desenvolvimento com agentes. O avaliador usa registros reais de telemetria. A economia de tokens ainda não foi medida.

[Decisões de engenharia](docs/engineering.md) · [Licença MIT](LICENSE)
