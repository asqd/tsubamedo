use std::path::{Path, PathBuf};

use super::*;
use crate::collection::Collection;
use crate::model::{ApiKeyLocation, Auth, Check, ClientAuth, Grant, OAuth2, Part};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/import").join(name)
}

fn find<'a>(i: &'a Imported, name: &str) -> &'a Placed {
    i.requests.iter().find(|p| p.request.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("нет запроса {name}"))
}

fn var<'a>(list: &'a [Var], name: &str) -> &'a Var {
    list.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("нет переменной {name}"))
}

fn warned(i: &Imported, part: &str) -> bool {
    i.warnings.iter().any(|w| w.contains(part))
}

#[test]
fn insomnia_v4_with_response_tags_folders_and_environments() {
    let i = read(&fixture("insomnia-v4.json")).unwrap();
    assert_eq!((i.format, i.name.as_str()), (Format::Insomnia, "Магазин"));
    let order: Vec<(&[String], &str)> = i.requests.iter().map(|p| (p.folder.as_slice(), p.request.name.as_deref().unwrap())).collect();
    let orders = ["Заказы".to_string()];
    assert_eq!(
        order,
        [(&[][..], "Войти"), (&orders[..], "Получить заказ"), (&orders[..], "Создать заказ"), (&orders[..], "Загрузить фото"), (&[][..], "/hello")]
    );

    // `{% response %}` became a capture in the request it reads, and a variable where it was.
    let login = &find(&i, "Войти").request;
    assert_eq!(login.captures, [Pair::new("x_request_id", "header X-Request-Id"), Pair::new("token", "$.token")]);
    assert_eq!(login.auth, Auth::None);
    assert!(login.headers.is_empty(), "Content-Type JSON-тела не нужен: {:?}", login.headers);
    let create = &find(&i, "Создать заказ").request;
    assert_eq!(create.auth, Auth::Bearer { token: "{{token}}".into() }, "авторизация папки");
    assert_eq!(create.url, "{{base_url}}/{{api.version}}/orders");
    assert_eq!(create.query, [Pair::new("src", "insomnia"), Pair { name: "dry".into(), value: "true".into(), enabled: false }]);
    assert_eq!(create.body, Body::Json { content: r#"{"id": "{{$uuid}}", "at": "{{$isoTimestamp}}"}"#.into() });
    assert!(create.scripts.before.starts_with("insomnia.request.addHeader"));
    assert!(create.scripts.after.contains("insomnia.environment.set('order_id'"));
    assert_eq!(find(&i, "Получить заказ").request.headers, [Pair::new("X-Trace", "{{x_request_id}}")]);

    let photo = &find(&i, "Загрузить фото").request;
    assert_eq!(photo.auth, Auth::ApiKey { key: "X-Key".into(), value: "{{secret_key}}".into(), location: ApiKeyLocation::Header });
    assert!(photo.headers.is_empty(), "фиксированный multipart/form-data сломал бы границу");
    assert_eq!(
        photo.body,
        Body::Multipart {
            parts: vec![
                Part { name: "title".into(), value: "Котик".into(), is_file: false, enabled: true },
                Part { name: "photo".into(), value: "/Users/me/kot.jpg".into(), is_file: true, enabled: true },
            ]
        }
    );

    assert_eq!(var(&i.vars, "api.version").value, "v1");
    assert!(var(&i.vars, "secret_key").secret, "по имени похоже на секрет");
    assert!(!var(&i.vars, "base_url").secret);
    let prod = i.environments.iter().find(|e| e.name == "prod").unwrap();
    assert!(prod.vars.iter().all(|v| v.secret), "приватное окружение Insomnia");
    let hello = i.requests.iter().find(|p| p.request.url.ends_with("/hello")).unwrap();
    assert_eq!(
        hello.request.auth,
        Auth::OAuth2(OAuth2 {
            grant: Grant::ClientCredentials,
            token_url: "{{base_url}}/oauth/token".into(),
            client_id: "app".into(),
            client_secret: "{{secret_key}}".into(),
            scope: "read".into(),
            pkce: false,
            client_auth: ClientAuth::Body,
            ..OAuth2::default()
        })
    );
    assert!(warned(&i, "gRPC-запросы (1)"));
    assert!(!warned(&i, "не заданы"), "{:?}", i.warnings);
}

#[test]
fn insomnia_v5_yaml() {
    let i = read(&fixture("insomnia-v5.yaml")).unwrap();
    assert_eq!(i.name, "Котики");
    let all = &find(&i, "Все котики").request;
    assert_eq!(all.auth, Auth::Basic { username: "{{user}}".into(), password: "{{password}}".into() });
    assert_eq!(all.options.follow_redirects, Some(false));
    assert_eq!(all.scripts.before, "// из папки «Котики»\nconsole.log(\"папка\")\n");
    assert!(all.scripts.after.contains("to.have.lengthOf(2)"));
    let new = &find(&i, "Новый котик").request;
    assert_eq!(new.body, Body::Form { fields: vec![Pair::new("name", "Барсик"), Pair { name: "age".into(), value: "2".into(), enabled: false }] });
    assert_eq!(find(&i, "GraphQL").request.body, Body::Json { content: r#"{"query":"{ cats { name } }"}"#.into() });
    let local = &i.environments[0];
    assert_eq!((local.name.as_str(), var(&local.vars, "base_url").value.as_str()), ("local", "http://localhost:3000"));
}

#[test]
fn postman_collection_with_inherited_auth_and_scripts() {
    let i = read(&fixture("postman.json")).unwrap();
    assert_eq!((i.format, i.name.as_str()), (Format::Postman, "Postman API"));
    // An empty placeholder for what «Вход» sets would keep «Вход» from being sent first.
    assert!(!i.vars.iter().any(|v| v.name == "token"));
    assert!(!i.vars.iter().any(|v| v.name == "old"), "выключенная переменная");

    let login = &find(&i, "Вход").request;
    assert_eq!(login.auth, Auth::None, "noauth у самого запроса");
    assert_eq!(login.body, Body::Json { content: r#"{"login": "kot"}"#.into() });
    assert_eq!(login.scripts.before, "// из коллекции\npm.variables.set(\"ts\", Date.now());\n");
    assert!(login.scripts.after.contains("pm.collectionVariables.set(\"token\""));

    let user = find(&i, "Получить пользователя");
    assert_eq!(user.folder, ["Users / пользователи"]);
    assert_eq!(user.request.auth, Auth::Bearer { token: "{{token}}".into() }, "авторизация коллекции");
    assert_eq!(user.request.url, "{{base_url}}/users/42", "переменная пути :id");
    assert_eq!(
        user.request.query,
        [Pair::new("expand", "orders"), Pair { name: "debug".into(), value: "1".into(), enabled: false }, Pair::new("rid", "{{$uuid}}")]
    );

    let avatar = &find(&i, "Аватар").request;
    assert_eq!(avatar.auth, Auth::ApiKey { key: "X-Api-Key".into(), value: "{{api_key}}".into(), location: ApiKeyLocation::Query });
    let Body::Multipart { parts } = &avatar.body else { panic!("{:?}", avatar.body) };
    assert_eq!((parts[1].value.as_str(), parts[1].is_file), ("/tmp/a.png", true));

    let graphql = &find(&i, "GraphQL").request;
    let Body::Json { content } = &graphql.body else { panic!() };
    assert_eq!(serde_json::from_str::<Value>(content).unwrap(), serde_json::json!({"query": "query { me { id } }", "variables": {"a": 1}}));
    assert_eq!(find(&i, "XML").request.body, Body::Text { content: "<a/>".into(), content_type: Some("application/xml".into()) });
    assert_eq!(graphql.auth, Auth::Digest { username: "кот".into(), password: "{{digest_password}}".into() });
    assert_eq!(parts[0].value, "{{$randomFirstName}}", "у нас есть все переменные Postman");
    assert!(!warned(&i, "{{$randomFirstName}}"));
    assert!(warned(&i, "{{$randomMagic}}"));
    assert!(warned(&i, "не заданы переменные: api_key, digest_password"));
}

#[test]
fn postman_environment() {
    let i = read(&fixture("postman-env.json")).unwrap();
    assert_eq!(i.format, Format::PostmanEnvironment);
    assert!(i.requests.is_empty());
    let env = &i.environments[0];
    assert_eq!(env.name, "Stage");
    assert_eq!(env.vars.iter().map(|v| (v.name.as_str(), v.secret)).collect::<Vec<_>>(), [("base_url", false), ("client", true)]);
}

#[test]
fn openapi_operations_with_examples_and_security() {
    let i = read(&fixture("openapi.yaml")).unwrap();
    assert_eq!((i.format, i.name.as_str()), (Format::OpenApi, "Зоомагазин"));
    assert_eq!(var(&i.vars, "base_url").value, "https://eu.pets.test/v1", "переменные сервера подставлены");
    let envs: Vec<(&str, &str)> = i.environments.iter().map(|e| (e.name.as_str(), e.vars[0].value.as_str())).collect();
    assert_eq!(envs, [("Прод", "https://eu.pets.test/v1"), ("Локально", "http://localhost:8080/v1")]);

    let list = find(&i, "Список питомцев");
    assert_eq!(list.folder, ["pets"]);
    assert_eq!(list.request.url, "{{base_url}}/pets");
    assert_eq!(list.request.query, [Pair { name: "limit".into(), value: "20".into(), enabled: false }, Pair::new("kind", "cat")]);
    assert_eq!(list.request.auth, Auth::Bearer { token: "{{token}}".into() });
    assert_eq!(list.request.checks, [Check::new("status == 200"), Check::new("$ schema schemas/GET pets ответ.json")]);
    assert_eq!(find(&i, "Один питомец").request.checks[1], Check::new("$ schema schemas/Pet.json"));
    assert_eq!(
        find(&i, "Один питомец").request.auth,
        Auth::OAuth2(OAuth2 {
            grant: Grant::ClientCredentials,
            token_url: "https://id.pets.test/token".into(),
            client_id: "{{client_id}}".into(),
            client_secret: "{{client_secret}}".into(),
            scope: "pets:read owners:read".into(),
            ..OAuth2::default()
        }),
        "клиент сам по себе — client credentials"
    );
    let files: Vec<&str> = i.files.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(files, ["schemas/GET pets ответ.json", "schemas/Pet.json", "schemas/Owner.json"]);
    let pet: Value = serde_json::from_str(&i.files[1].1).unwrap();
    assert_eq!(pet["properties"]["born"]["type"], serde_json::json!(["string", "null"]), "nullable");
    assert_eq!((pet["properties"]["weight"]["exclusiveMinimum"].clone(), pet["properties"]["weight"].get("minimum")), (serde_json::json!(0), None));
    assert_eq!(pet["properties"]["parent"], serde_json::json!({"$ref": "Pet.json"}));

    let create = &find(&i, "createPet").request;
    let Body::Json { content } = &create.body else { panic!("{:?}", create.body) };
    let body: Value = serde_json::from_str(content).unwrap();
    assert_eq!(body["name"], "Барсик");
    assert!(body.get("id").is_none(), "readOnly не отправляется");
    assert_eq!(body["owner"], serde_json::json!({"email": "user@example.com", "vip": true}), "allOf");
    assert_eq!(body["parent"], serde_json::json!({}), "цикл схем оборван");
    assert_eq!(create.checks, [Check::new("status == 201")]);

    assert_eq!(find(&i, "Один питомец").request.url, "{{base_url}}/pets/{{petId}}");
    let delete = i.requests.iter().find(|p| p.request.method == "DELETE").unwrap();
    assert_eq!((delete.request.url.as_str(), &delete.request.auth), ("{{base_url}}/pets/7", &Auth::None), "security: [] и пример параметра");
    let photo = i.requests.iter().find(|p| p.request.url.ends_with("/photo")).unwrap();
    assert_eq!(photo.request.url, "{{base_url}}/pets/3/photo", "параметр по $ref");
    assert_eq!(photo.request.auth, Auth::ApiKey { key: "X-Api-Key".into(), value: "{{api_key}}".into(), location: ApiKeyLocation::Header });
    assert!(matches!(&photo.request.body, Body::Multipart { parts } if parts[1].is_file && parts[0].value == "Мурзик"));
    assert!(find(&i, "/health").folder.is_empty());
    assert!(warned(&i, "не заданы переменные: api_key, client_id, client_secret, petId, token"));
}

#[test]
fn swagger_2() {
    let i = read(&fixture("swagger.json")).unwrap();
    assert_eq!(var(&i.vars, "base_url").value, "https://old.test/api");
    let create = &find(&i, "Создать").request;
    assert_eq!(create.auth, Auth::Basic { username: "{{username}}".into(), password: "{{password}}".into() });
    assert_eq!(create.body, Body::Json { content: "{\n  \"name\": \"string\",\n  \"age\": 0\n}".into() });
    assert_eq!(find(&i, "/login").request.body, Body::Form { fields: vec![Pair::new("user", ""), Pair::new("remember", "true")] });
}

#[test]
fn har_keeps_the_api_calls() {
    let i = read(&fixture("browser.har")).unwrap();
    let got: Vec<(&str, &str, &str)> =
        i.requests.iter().map(|p| (p.folder.first().map_or("", String::as_str), p.request.method.as_str(), p.request.url.as_str())).collect();
    assert_eq!(
        got,
        [
            ("api.shop.test", "GET", "https://api.shop.test/cart"),
            ("api.shop.test", "POST", "https://api.shop.test/cart/items"),
            ("shop.test", "POST", "https://shop.test/search"),
        ]
    );
    let cart = &i.requests[0].request;
    assert_eq!(cart.query, [Pair::new("user", "42"), Pair::new("q", "кот")]);
    assert_eq!(cart.headers, [Pair::new("accept", "application/json"), Pair::new("cookie", "sid=abc")], "служебные заголовки браузера убраны");
    assert!(i.requests[1].request.headers.is_empty());
    assert_eq!(i.requests[2].request.body, Body::Form { fields: vec![Pair::new("q", "кот"), Pair::new("page", "2")] });
    assert!(warned(&i, "Cookie"));
    assert!(warned(&i, "пропущено записей: 5"));
}

#[test]
fn unknown_and_broken_files_say_why() {
    assert!(parse("{\"hello\": 1}").unwrap_err().to_string().contains("не похоже"));
    assert!(parse("{\"hello\": ").unwrap_err().to_string().contains("не JSON"));
    assert!(parse("type: spec.insomnia.rest/5.0\nname: x\n").unwrap_err().to_string().contains("OpenAPI"));
}

#[test]
fn writes_a_new_collection_that_opens() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("shop");
    let i = read(&fixture("insomnia-v4.json")).unwrap();
    let w = write(&i, &root).unwrap();
    assert_eq!((w.folder.as_str(), w.requests), ("", 5));
    assert_eq!(w.environments, ["stage", "prod"]);

    let c = Collection::open(&root).unwrap();
    assert!(c.broken.is_empty(), "{:?}", c.broken);
    assert_eq!(c.name, "Магазин");
    // The order of Insomnia stays in the names where it was not alphabetical.
    let ids: Vec<&str> = c.requests.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["01 Войти", "02 hello", "Заказы/01 Получить заказ", "Заказы/02 Создать заказ", "Заказы/03 Загрузить фото"]);
    assert_eq!(c.find("02 hello").unwrap().request.name.as_deref(), Some("/hello"));
    assert_eq!(c.find("01 Войти").unwrap().request.captures.len(), 2);

    let prod = c.vars(Some("prod"), &Vars::new()).unwrap();
    assert_eq!((prod["base_url"].as_str(), prod["api.version"].as_str(), prod["password"].as_str()), ("https://shop.test", "v1", "hunter2"));
    let secrets = fs::read_to_string(root.join("tsubamedo.local.toml")).unwrap();
    assert!(secrets.contains("secret_key = \"s3cr3t\""), "{secrets}");
    assert!(!fs::read_to_string(root.join(MANIFEST)).unwrap().contains("s3cr3t"));
    assert_eq!(fs::read_to_string(root.join("env/prod.toml")).unwrap(), "[vars]\n", "все значения prod — секреты");
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), "*.local.toml\n");
}

#[test]
fn openapi_schemas_check_real_answers() {
    use crate::checks::{Answer, run_checks};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("zoo");
    write(&read(&fixture("openapi.yaml")).unwrap(), &root).unwrap();
    let c = Collection::open(&root).unwrap();
    let entry = c.requests.iter().find(|e| e.request.name.as_deref() == Some("Список питомцев")).unwrap();
    let check = |body: &str| {
        let r = crate::http::Response {
            status: 200,
            reason: "OK".into(),
            version: "HTTP/1.1".into(),
            headers: Vec::new(),
            body: crate::http::ResponseBody::Memory(bytes::Bytes::from(body.to_string())),
            wire_size: 0,
            timings: Default::default(),
            url: String::new(),
            remote_addr: "127.0.0.1:80".parse().unwrap(),
            tls: None,
            redirects: Vec::new(),
            sent_cookie: None,
            via: None,
        };
        let out = run_checks(&entry.request.checks, &Answer::new(&r), &Vars::new(), Some(entry.dir()));
        (out[1].ok, out[1].error.clone().unwrap_or_default())
    };
    let good = r#"[{"id": 1, "name": "Барсик", "born": null, "weight": 4.5, "owner": {"email": "a@b.c", "vip": false}, "parent": {"name": "Мурка"}}]"#;
    assert_eq!(check(good), (true, String::new()));
    let bad = r#"[{"name": 5, "weight": 0}, {"name": "Рыжик", "parent": {"name": "Мурка", "born": 1}}]"#;
    assert_eq!(check(bad).1, "$[0].name: ожидалось строка, а здесь число; $[0].weight: 0 не больше 0; $[1].parent.born: ожидалось null или строка, а здесь число");
}

#[test]
fn writes_into_an_open_collection_keeping_its_values() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join(MANIFEST), "name = \"Мои\"\n\n[vars]\nbase_url = \"https://mine.test\"\n").unwrap();
    fs::create_dir_all(root.join("env")).unwrap();
    fs::write(root.join("env/stage.toml"), "# мой стенд\n[vars]\nbase_url = \"https://stage.mine\"\n").unwrap();

    let w = write(&read(&fixture("postman.json")).unwrap(), root).unwrap();
    assert_eq!(w.folder, "Postman API");
    assert_eq!(w.kept, ["base_url (коллекция)"]);
    let w = write(&read(&fixture("postman-env.json")).unwrap(), root).unwrap();
    assert_eq!((w.kept.as_slice(), w.environments.as_slice()), (&["base_url (stage)".to_string()][..], &["stage".to_string()][..]), "имя окружения коллекции");
    assert_eq!(w.requests, 0);

    let c = Collection::open(root).unwrap();
    assert!(c.broken.is_empty(), "{:?}", c.broken);
    let ids: Vec<&str> = c.requests.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "Postman API/01 Вход",
            "Postman API/02 GraphQL",
            "Postman API/03 XML",
            "Postman API/Users пользователи/01 Получить пользователя",
            "Postman API/Users пользователи/02 Аватар",
            "Postman API/Users пользователи/03 Форма",
        ]
    );
    let stage = fs::read_to_string(root.join("env/stage.toml")).unwrap();
    assert!(stage.starts_with("# мой стенд\n") && stage.contains("https://stage.mine") && !stage.contains("stage.api.test"), "{stage}");
    assert!(fs::read_to_string(root.join("env/stage.local.toml")).unwrap().contains("client = \"c-1\""));
    assert_eq!(Collection::open(root).unwrap().environments(), ["stage"]);
    // Importing again goes next to the first, not over it.
    let again = write(&read(&fixture("postman.json")).unwrap(), root).unwrap();
    assert_eq!(again.folder, "Postman API 2");
}

#[test]
fn aws_signature_from_postman_and_insomnia() {
    let dir = tempfile::tempdir().unwrap();
    let want = Auth::AwsV4(crate::model::AwsV4 {
        access_key: "AK".into(),
        secret_key: "{{aws_secret}}".into(),
        session_token: String::new(),
        region: "eu-west-1".into(),
        service: "s3".into(),
    });
    let postman = dir.path().join("postman.json");
    std::fs::write(
        &postman,
        r#"{"info": {"name": "AWS", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
            "item": [{"name": "S3", "request": {"method": "GET", "url": "https://b.s3.amazonaws.com/k",
              "auth": {"type": "awsv4", "awsv4": [{"key": "accessKey", "value": "AK"}, {"key": "secretKey", "value": "{{aws_secret}}"},
                                                  {"key": "region", "value": "eu-west-1"}, {"key": "service", "value": "s3"}]}}}]}"#,
    )
    .unwrap();
    assert_eq!(find(&read(&postman).unwrap(), "S3").request.auth, want);
    let insomnia = dir.path().join("insomnia.json");
    std::fs::write(
        &insomnia,
        r#"{"_type": "export", "__export_format": 4, "resources": [
            {"_id": "wrk_1", "_type": "workspace", "name": "AWS"},
            {"_id": "req_1", "_type": "request", "parentId": "wrk_1", "name": "S3", "method": "GET", "url": "https://b.s3.amazonaws.com/k",
             "authentication": {"type": "iam", "accessKeyId": "AK", "secretAccessKey": "{{ _.aws_secret }}", "region": "eu-west-1", "service": "s3"}}]}"#,
    )
    .unwrap();
    assert_eq!(find(&read(&insomnia).unwrap(), "S3").request.auth, want);
}

#[test]
fn openapi_split_into_files() {
    let dir = tempfile::tempdir().unwrap();
    let put = |path: &str, text: &str| {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    put(
        "openapi.yaml",
        r#"
openapi: 3.0.3
info: {title: Зоомагазин, version: "1"}
servers: [{url: "https://zoo.test/api"}]
paths:
  /pets/{id}:
    get:
      summary: Питомец
      parameters:
        - $ref: 'common/params.yaml#/PetId'
      responses:
        '200':
          description: питомец
          content:
            application/json:
              schema: {$ref: 'schemas/pet.yaml'}
        default: {$ref: 'common/responses.yaml#/Error'}
  /orders: {$ref: 'paths/orders.yaml'}
  /remote:
    get:
      summary: Далеко
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {$ref: 'https://example.com/schemas/x.json'}
components:
  schemas:
    Tag: {type: string, example: кот}
"#,
    );
    put("common/params.yaml", "PetId:\n  name: id\n  in: path\n  required: true\n  schema: {type: integer, example: 7}\n");
    put("common/responses.yaml", "Error:\n  description: ошибка\n  content:\n    application/json:\n      schema: {$ref: '../schemas/error.json'}\n");
    put(
        "schemas/pet.yaml",
        r#"
type: object
required: [id, owner]
properties:
  id: {type: integer}
  name: {type: string, example: Мурка}
  default: {type: boolean}
  owner: {$ref: './owner.yaml#/Owner'}
  tags: {type: array, items: {$ref: '../openapi.yaml#/components/schemas/Tag'}}
"#,
    );
    put("schemas/owner.yaml", "Owner:\n  type: object\n  properties:\n    name: {type: string, example: Аня}\n    pets: {type: array, items: {$ref: 'pet.yaml'}}\n");
    put("schemas/error.json", r#"{"type": "object", "properties": {"message": {"type": "string"}}}"#);
    put(
        "paths/orders.yaml",
        "post:\n  summary: Заказ\n  requestBody:\n    content:\n      application/json:\n        schema: {$ref: '../schemas/pet.yaml'}\n  responses:\n    '201': {description: создан}\n",
    );

    let i = read(&dir.path().join("openapi.yaml")).unwrap();
    let pet = &find(&i, "Питомец").request;
    assert_eq!(pet.url, "{{base_url}}/pets/7", "параметр из другого файла");
    assert_eq!(pet.checks, [Check::new("status == 200"), Check::new("$ schema schemas/pet.json")]);

    let files: BTreeMap<&str, Value> = i.files.iter().map(|(p, t)| (p.as_str(), serde_json::from_str(t).unwrap())).collect();
    assert_eq!(files["schemas/pet.json"]["properties"]["owner"], serde_json::json!({"$ref": "Owner.json"}));
    assert_eq!(files["schemas/pet.json"]["properties"]["tags"]["items"], serde_json::json!({"$ref": "Tag.json"}), "обратно в главный файл");
    assert_eq!(files["schemas/pet.json"]["properties"]["default"], serde_json::json!({"type": "boolean"}), "свойство с именем default");
    assert_eq!(files["schemas/Owner.json"]["properties"]["pets"]["items"], serde_json::json!({"$ref": "pet.json"}), "круг между файлами");

    let order = &find(&i, "Заказ").request;
    let Body::Json { content } = &order.body else { panic!("{:?}", order.body) };
    let body: Value = serde_json::from_str(content).unwrap();
    assert_eq!((body["name"].as_str(), body["owner"]["name"].as_str(), body["tags"][0].as_str()), (Some("Мурка"), Some("Аня"), Some("кот")));

    assert!(warned(&i, "ссылки по сети не загружаются: https://example.com/schemas/x.json"), "{:?}", i.warnings);
    assert!(!i.warnings.iter().any(|w| w.contains("не прочитана") || w.contains("никуда")), "{:?}", i.warnings);

    // The same text pasted has no folder to read the files from.
    let pasted = parse(&std::fs::read_to_string(dir.path().join("openapi.yaml")).unwrap()).unwrap();
    assert!(warned(&pasted, "импортируйте само описание файлом"), "{:?}", pasted.warnings);
}
