//! `tsubamedo`: requests and collections from the terminal and CI.

mod format;

use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use tsubamedo_core::collection::{Collection, MANIFEST};
use tsubamedo_core::http::{Response, SendOptions};
use tsubamedo_core::prepare::prepare;
use tsubamedo_core::request_file::RequestFile;
use tsubamedo_core::run::{Event, Exchange, RunOptions, Runner};
use tsubamedo_core::{Body, Error, Pair, Request, Vars};

mod junit;

use format::{Paint, duration, size};

#[derive(Parser)]
#[command(name = "tsubamedo", version, about = "TsubameDo: HTTP-запросы и коллекции в терминале")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Отправить один запрос: адрес, файл запроса или его имя в коллекции.
    Send(SendArgs),
    /// Отправить запросы коллекции: каждый после тех, чьи значения он берёт, с проверками.
    Run(RunArgs),
    /// Проверить, что файлы читаются и все переменные заданы.
    Check(CheckArgs),
    /// Перенести коллекцию Insomnia или Postman, описание OpenAPI или Swagger, запись HAR.
    Import {
        /// Экспорт Insomnia (JSON или YAML), коллекция или окружение Postman, OpenAPI, HAR.
        file: PathBuf,
        /// Куда: новая папка (по умолчанию — по имени коллекции в текущей) или существующая
        /// коллекция — тогда запросы лягут в папку внутри неё.
        #[arg(short, long, value_name = "ПАПКА")]
        to: Option<PathBuf>,
    },
    /// Создать коллекцию в папке.
    Init {
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
}

#[derive(Args)]
struct Common {
    /// Окружение из папки env коллекции.
    #[arg(short, long)]
    env: Option<String>,
    /// Переменная поверх всех остальных: --var token=abc.
    #[arg(long = "var", value_name = "ИМЯ=ЗНАЧЕНИЕ", value_parser = parse_var)]
    vars: Vec<(String, String)>,
    /// Не проверять сертификат сервера (тестовые стенды с самоподписанным).
    #[arg(short = 'k', long)]
    insecure: bool,
    /// Ограничение на весь обмен, в секундах.
    #[arg(long, value_name = "СЕК")]
    max_time: Option<f64>,
}

#[derive(Args)]
struct SendArgs {
    target: String,
    #[command(flatten)]
    common: Common,
    #[arg(short = 'X', long)]
    method: Option<String>,
    /// Заголовок «Имя: значение».
    #[arg(short = 'H', long = "header")]
    headers: Vec<String>,
    /// Тело как в curl (form-urlencoded); @файл — содержимое файла.
    #[arg(short, long)]
    data: Option<String>,
    /// Тело JSON; @файл — из файла.
    #[arg(long)]
    json: Option<String>,
    /// Показать заголовки ответа.
    #[arg(short, long)]
    include: bool,
    /// Ничего не писать в stderr.
    #[arg(short, long)]
    quiet: bool,
    /// Не переходить по перенаправлениям.
    #[arg(long)]
    no_follow: bool,
    /// Сохранить тело ответа в файл.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Тело как пришло, без форматирования JSON.
    #[arg(long)]
    raw: bool,
    /// Код выхода 1, если статус ответа 400 и выше (проверки из файла запроса — всегда).
    #[arg(short, long)]
    fail: bool,
}

#[derive(Args)]
struct RunArgs {
    #[arg(default_value = ".")]
    dir: PathBuf,
    #[command(flatten)]
    common: Common,
    /// Только запросы, в имени или пути которых есть этот текст.
    #[arg(long)]
    filter: Option<String>,
    /// Считать ошибкой статус 400 и выше.
    #[arg(short, long)]
    fail: bool,
    /// Остановиться на первой ошибке.
    #[arg(long)]
    bail: bool,
    /// Сколько запросов отправлять одновременно; те, что ждут значений из других, всё равно
    /// идут после них.
    #[arg(short, long, value_name = "N", default_value_t = 1)]
    parallel: usize,
    /// Сколько раз повторить упавший запрос.
    #[arg(long, value_name = "N", default_value_t = 0)]
    retry: u32,
    /// Записать отчёт JUnit XML для CI (GitLab, Jenkins, GitHub Actions).
    #[arg(long, value_name = "ФАЙЛ")]
    junit: Option<PathBuf>,
}

#[derive(Args)]
struct CheckArgs {
    #[arg(default_value = ".")]
    dir: PathBuf,
    /// Окружение; без него проверяются все.
    #[arg(short, long)]
    env: Option<String>,
}

fn parse_var(text: &str) -> Result<(String, String), String> {
    let (name, value) = text.split_once('=').ok_or("нужно ИМЯ=ЗНАЧЕНИЕ")?;
    Ok((name.trim().to_string(), value.to_string()))
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Send(args) => send_one(args).await,
        Command::Run(args) => run(args).await,
        Command::Check(args) => check(args),
        Command::Import { file, to } => import(&file, to),
        Command::Init { dir } => init(&dir),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{}", Paint::stderr().red(&format!("ошибка: {e}")));
            if let Error::Tls { reason, .. } = &e
                && reason.contains("certificate")
            {
                eprintln!("Если это тестовый стенд с самоподписанным сертификатом, добавьте -k.");
            }
            ExitCode::FAILURE
        }
    }
}

fn send_options(common: &Common) -> SendOptions {
    SendOptions {
        verify_tls: !common.insecure,
        timeout: common.max_time.map(Duration::from_secs_f64),
        ..SendOptions::default()
    }
}

fn overrides(common: &Common) -> Vars {
    common.vars.iter().cloned().collect()
}

/// What `send` was pointed at: the request, its collection and id there, and the folder for
/// its relative paths.
struct Target {
    request: Request,
    collection: Option<Collection>,
    id: Option<String>,
    dir: PathBuf,
}

fn resolve_target(args: &SendArgs) -> tsubamedo_core::Result<Target> {
    let target = Path::new(&args.target);
    let cwd = std::env::current_dir().map_err(|e| Error::io(".", e))?;
    let from_file = target.is_file() && target.extension().is_some_and(|e| e == "toml");
    let root = Collection::find_root(if from_file { target } else { &cwd });
    let collection = root.map(Collection::open).transpose()?;
    if collection.is_none() && args.common.env.is_some() {
        return Err(Error::Invalid(format!("окружение задано, но {MANIFEST} не найден ни здесь, ни выше")));
    }
    if from_file {
        let request = RequestFile::open(target)?.request()?;
        let dir = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
        let file = target.canonicalize().ok();
        let id = collection.as_ref().and_then(|c| c.requests.iter().find(|e| e.path.canonicalize().ok() == file)).map(|e| e.id.clone());
        return Ok(Target { request, collection, id, dir });
    }
    if let Some(entry) = collection.as_ref().and_then(|c| c.find(args.target.trim_end_matches(".toml"))) {
        let (request, id, dir) = (entry.request.clone(), Some(entry.id.clone()), entry.dir().to_path_buf());
        return Ok(Target { request, collection, id, dir });
    }
    Ok(Target { request: Request { url: args.target.clone(), ..Request::default() }, collection, id: None, dir: cwd })
}

fn data_arg(value: &str) -> io::Result<String> {
    match value.strip_prefix('@') {
        Some("-") => {
            let mut text = String::new();
            io::stdin().read_to_string(&mut text)?;
            Ok(text)
        }
        Some(path) => std::fs::read_to_string(path),
        None => Ok(value.to_string()),
    }
}

async fn send_one(args: SendArgs) -> tsubamedo_core::Result<ExitCode> {
    let Target { mut request, collection, id, dir } = resolve_target(&args)?;
    for header in &args.headers {
        let (name, value) = header
            .split_once(':')
            .ok_or_else(|| Error::Invalid(format!("заголовок «{header}»: нужно «Имя: значение»")))?;
        request.headers.retain(|p| !p.name.eq_ignore_ascii_case(name.trim()));
        request.headers.push(Pair::new(name.trim(), value.trim()));
    }
    if let Some(json) = &args.json {
        request.body = Body::Json { content: data_arg(json).map_err(|e| Error::io(json, e))? };
    } else if let Some(data) = &args.data {
        let content_type = Some("application/x-www-form-urlencoded".to_string());
        request.body = Body::Text { content: data_arg(data).map_err(|e| Error::io(data, e))?, content_type };
    }
    if let Some(method) = &args.method {
        request.method = method.clone();
    } else if request.method == "GET" && (args.json.is_some() || args.data.is_some()) {
        request.method = "POST".into();
    }
    if args.no_follow {
        request.options.follow_redirects = Some(false);
    }

    let mut runner = Runner::new(collection.as_ref(), args.common.env.as_deref(), overrides(&args.common), Vars::new(), send_options(&args.common))?;
    let mut exchanges = runner.send(id.as_deref(), &request, &dir).await;
    let last = exchanges.pop().expect("the request itself is always there");
    let paint = Paint::stderr();
    if !args.quiet {
        for ex in &exchanges {
            let status = ex.result.as_ref().map_or_else(|e| paint.red(&e.to_string()), |r| paint.status(r.status, &r.status.to_string()));
            eprintln!("{}", paint.dim(&format!("Сначала {}: {status}", ex.id.as_deref().unwrap_or_default())));
        }
    }
    let passed = last.passed();
    let lines = check_lines(&last, &paint);
    let response = last.result?;
    if !args.quiet {
        print_head(&response, args.include);
        for line in lines {
            eprintln!("{line}");
        }
    }
    write_body(&response, &args)?;
    Ok(if !passed || (args.fail && response.status >= 400) { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

/// Under a request: what its scripts logged, its checks, and what it captured or could not.
fn check_lines(ex: &Exchange, paint: &Paint) -> Vec<String> {
    let mut out: Vec<String> = ex.logs.iter().map(|l| format!("  {}", paint.dim(&format!("› {l}")))).collect();
    for c in &ex.checks {
        if c.ok {
            out.push(format!("  {} {}", paint.green("✓"), paint.dim(&c.check)));
        } else {
            let why = c.error.clone().or_else(|| c.actual.as_ref().map(|a| format!("пришло {a}"))).unwrap_or_default();
            out.push(format!("  {} {} {}", paint.red("✗"), c.check, paint.red(&format!("— {why}"))));
        }
    }
    for c in &ex.captures {
        match &c.error {
            None => out.push(format!("  {}", paint.dim(&format!("→ {}", c.name)))),
            Some(e) => out.push(format!("  {} {}", paint.red("✗"), paint.red(&format!("{}: {e}", c.name)))),
        }
    }
    out
}

fn print_head(r: &Response, include: bool) {
    let paint = Paint::stderr();
    let mut err = io::stderr().lock();
    for hop in &r.redirects {
        let _ = writeln!(err, "{}", paint.dim(&format!("→ {} {}", hop.status, hop.url)));
    }
    let status = format!("{} {} {}", r.version, r.status, r.reason);
    let _ = writeln!(err, "{}", paint.status(r.status, &status));
    if include {
        for (name, value) in &r.headers {
            let _ = writeln!(err, "{}: {value}", paint.bold(name));
        }
    }
    let t = &r.timings;
    let mut parts = vec![format!("DNS {}", duration(t.dns)), format!("подключение {}", duration(t.connect))];
    if let Some(tls) = t.tls {
        parts.push(format!("TLS {}", duration(tls)));
    }
    parts.push(format!("ожидание {}", duration(t.wait)));
    parts.push(format!("загрузка {}", duration(t.download)));
    parts.push(format!("всего {}", duration(t.total)));
    let mut body = size(r.body.len());
    if r.wire_size != r.body.len() {
        body.push_str(&format!(" (по сети {})", size(r.wire_size)));
    }
    parts.push(body);
    let _ = writeln!(err, "{}", paint.dim(&parts.join(" · ")));
}

/// Pretty JSON for people, the bytes as they came for pipes and files.
fn write_body(r: &Response, args: &SendArgs) -> tsubamedo_core::Result<()> {
    if let Some(path) = &args.output {
        let mut file = std::fs::File::create(path).map_err(|e| Error::io(path, e))?;
        io::copy(&mut r.body.reader().map_err(|e| Error::io(path, e))?, &mut file).map_err(|e| Error::io(path, e))?;
        if !args.quiet {
            eprintln!("Сохранено: {} ({})", path.display(), size(r.body.len()));
        }
        return Ok(());
    }
    let stdout = io::stdout();
    let terminal = stdout.is_terminal();
    let io_err = |e: io::Error| Error::io("stdout", e);
    if terminal {
        const SHOW_LIMIT: u64 = 20 << 20;
        let head = r.body.head(8 << 10).map_err(io_err)?;
        if head.contains(&0) {
            eprintln!("Двоичный ответ, {}: сохраните его через -o файл.", size(r.body.len()));
            return Ok(());
        }
        if r.body.len() > SHOW_LIMIT {
            eprintln!("Ответ {}: сохраните его через -o файл или направьте в другую программу.", size(r.body.len()));
            return Ok(());
        }
        let mut text = Vec::new();
        r.body.reader().map_err(io_err)?.read_to_end(&mut text).map_err(io_err)?;
        let mut out = stdout.lock();
        if r.is_json() && !args.raw
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&text)
        {
            let _ = serde_json::to_writer_pretty(&mut out, &value);
            let _ = writeln!(out);
            return Ok(());
        }
        let _ = out.write_all(&text);
        if !text.is_empty() && !text.ends_with(b"\n") {
            let _ = writeln!(out);
        }
        return Ok(());
    }
    let copied = io::copy(&mut r.body.reader().map_err(io_err)?, &mut stdout.lock());
    match copied {
        // The reader on the other end of the pipe left (`| head`): not our error.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
        other => {
            other.map_err(io_err)?;
        }
    }
    Ok(())
}

async fn run(args: RunArgs) -> tsubamedo_core::Result<ExitCode> {
    let collection = Collection::open(&args.dir)?;
    let mut runner = Runner::new(Some(&collection), args.common.env.as_deref(), overrides(&args.common), Vars::new(), send_options(&args.common))?;
    let paint = Paint::stdout();
    let started = Instant::now();
    let mut report = junit::Report::new(&collection.name);

    for broken in &collection.broken {
        println!("{} {broken}", paint.red("✗"));
        report.broken(&broken.to_string());
    }
    let filter = args.filter.as_deref().map(str::to_lowercase);
    let wanted = |entry: &tsubamedo_core::collection::Entry| {
        filter.as_ref().is_none_or(|f| entry.id.to_lowercase().contains(f) || entry.display_name().to_lowercase().contains(f))
    };
    let steps = runner.plan(&collection, wanted);
    let options = RunOptions { parallel: args.parallel, retries: args.retry, fail_status: args.fail, bail: args.bail };
    // Printed as they finish; the report keeps the collection's order.
    let mut finished: Vec<(usize, Vec<Exchange>, Vec<Vec<String>>)> = Vec::new();
    runner
        .run(&steps, &options, |event| match event {
            Event::Started { step, attempt } if attempt > 1 => {
                println!("{}", paint.dim(&format!("↻ {} — попытка {attempt}", steps[step].id)));
            }
            Event::Started { .. } => {}
            Event::Finished { step, exchanges, .. } => {
                let mut failures = Vec::new();
                for ex in &exchanges {
                    let f = print_exchange(ex, &collection, &args, &paint);
                    failures.push(f);
                }
                finished.push((step, exchanges, failures));
            }
        })
        .await;
    finished.sort_by_key(|(step, ..)| *step);
    for (_, exchanges, failures) in &finished {
        for (ex, f) in exchanges.iter().zip(failures) {
            report.case(ex, f);
        }
    }
    let (passed, failed) = report.counts();
    let summary = format!("Успешно: {passed}, с ошибкой: {failed}, за {}", duration(started.elapsed()));
    println!("\n{}", if failed == 0 { paint.green(&summary) } else { paint.red(&summary) });
    if let Some(path) = &args.junit {
        std::fs::write(path, report.xml(started.elapsed())).map_err(|e| Error::io(path, e))?;
        println!("{}", paint.dim(&format!("Отчёт JUnit: {}", path.display())));
    }
    Ok(if failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// One line for the exchange, and under it what failed; returns what failed.
fn print_exchange(ex: &Exchange, collection: &Collection, args: &RunArgs, paint: &Paint) -> Vec<String> {
    let status = ex.result.as_ref().ok().map(|r| r.status);
    let mut failures = ex.failures();
    if args.fail && status.is_some_and(|s| s >= 400) {
        failures.push(format!("статус {}", status.unwrap_or_default()));
    }
    let ok = failures.is_empty();
    let entry = collection.find(ex.id.as_deref().unwrap_or_default());
    let title = match entry.and_then(|e| e.request.name.as_deref()) {
        Some(name) => format!("{}  {}", ex.id.as_deref().unwrap_or_default(), paint.dim(name)),
        None => ex.id.clone().unwrap_or_default(),
    };
    let mark = if ok { paint.green("✓") } else { paint.red("✗") };
    match &ex.result {
        Ok(r) => {
            let checks = if ex.checks.is_empty() {
                String::new()
            } else {
                paint.dim(&format!("  проверки {}/{}", ex.checks.iter().filter(|c| c.ok).count(), ex.checks.len()))
            };
            println!(
                "{mark} {}  {:>8}  {:>9}  {} {title}{checks}",
                paint.status(r.status, &r.status.to_string()),
                duration(r.timings.total),
                size(r.body.len()),
                ex.request.method.to_ascii_uppercase(),
            );
            for line in check_lines(ex, paint).into_iter().filter(|l| !ok || !l.contains('✓')) {
                println!("  {line}");
            }
            if args.fail && r.status >= 400 && ex.checks.is_empty() {
                println!("    {}", paint.red(&format!("статус {}", r.status)));
            }
        }
        Err(e) => println!("{mark} {title}\n    {}", paint.red(&e.to_string())),
    }
    failures
}

fn check(args: CheckArgs) -> tsubamedo_core::Result<ExitCode> {
    let collection = Collection::open(&args.dir)?;
    let paint = Paint::stdout();
    let mut problems = 0;
    for broken in &collection.broken {
        println!("{} {broken}", paint.red("✗"));
        problems += 1;
    }
    let envs: Vec<Option<String>> = match args.env {
        Some(env) => vec![Some(env)],
        None if collection.environments().is_empty() => vec![None],
        None => collection.environments().into_iter().map(Some).collect(),
    };
    for env in &envs {
        let label = env.as_deref().unwrap_or("без окружения");
        let vars = match collection.vars(env.as_deref(), &Vars::new()) {
            Ok(vars) => vars,
            Err(e) => {
                println!("{} {label}: {e}", paint.red("✗"));
                problems += 1;
                continue;
            }
        };
        for entry in &collection.requests {
            if let Err(e) = prepare(&entry.request, &vars, entry.dir()) {
                println!("{} {label}: {}: {e}", paint.red("✗"), entry.id);
                problems += 1;
            }
        }
    }
    let labels: Vec<&str> = envs.iter().map(|e| e.as_deref().unwrap_or("без окружения")).collect();
    if problems == 0 {
        println!("{}", paint.green(&format!("Запросов: {}, все готовы ({})", collection.requests.len(), labels.join(", "))));
        Ok(ExitCode::SUCCESS)
    } else {
        println!("\n{}", paint.red(&format!("Проблем: {problems}")));
        Ok(ExitCode::FAILURE)
    }
}

fn import(file: &Path, to: Option<PathBuf>) -> tsubamedo_core::Result<ExitCode> {
    let imported = tsubamedo_core::import::read(file)?;
    let paint = Paint::stdout();
    let dir = to.unwrap_or_else(|| tsubamedo_core::import::new_dir(&imported, Path::new(".")));
    let written = tsubamedo_core::import::write(&imported, &dir)?;
    println!("{} {} «{}»", paint.bold("Импорт:"), imported.format.title(), imported.name);
    let place = if written.folder.is_empty() {
        format!("Создана коллекция {}", dir.display())
    } else {
        format!("В коллекцию {} в папку «{}»", written.root.display(), written.folder)
    };
    let mut what = vec![format!("запросов: {}", written.requests)];
    if !written.environments.is_empty() {
        what.push(format!("окружения: {}", written.environments.join(", ")));
    }
    println!("{place} — {}", what.join(", "));
    for w in &imported.warnings {
        println!("{}", paint.yellow(&format!("! {w}")));
    }
    if !written.kept.is_empty() {
        println!("{}", paint.dim(&format!("Уже были, оставлены прежние значения: {}", written.kept.join(", "))));
    }
    if written.requests > 0 {
        let env = written.environments.first().map(|e| format!(" -e {e}")).unwrap_or_default();
        println!("{}", paint.dim(&format!("Попробуйте: tsubamedo run {}{env}", written.root.display())));
    }
    Ok(ExitCode::SUCCESS)
}

fn init(dir: &Path) -> tsubamedo_core::Result<ExitCode> {
    let manifest = dir.join(MANIFEST);
    if manifest.exists() {
        return Err(Error::Invalid(format!("{} уже есть", manifest.display())));
    }
    let name = std::path::absolute(dir)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "Коллекция".into());
    let files = [
        (MANIFEST.to_string(), format!("name = {name:?}\n\n# Переменные для всех окружений.\n[vars]\nbase_url = \"https://httpbin.org\"\n")),
        ("env/dev.toml".into(), "[vars]\nbase_url = \"https://httpbin.org\"\n".into()),
        (
            "env/dev.local.toml".into(),
            "# Секреты окружения: этот файл не попадает в git.\n[vars]\ntoken = \"замените-меня\"\n".into(),
        ),
        (".gitignore".into(), "*.local.toml\n".into()),
    ];
    for (rel, text) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        if !path.exists() {
            std::fs::write(&path, text).map_err(|e| Error::io(&path, e))?;
        }
    }
    let example = Request {
        name: Some("Пример".into()),
        url: "{{base_url}}/anything".into(),
        query: vec![Pair::new("from", "tsubamedo")],
        headers: vec![Pair::new("Accept", "application/json"), Pair::new("Authorization", "Bearer {{token}}")],
        ..Request::default()
    };
    RequestFile::create(dir.join("example.toml"), &example).save()?;
    println!("Коллекция создана в {}. Попробуйте: tsubamedo run {} -e dev", dir.display(), dir.display());
    Ok(ExitCode::SUCCESS)
}
