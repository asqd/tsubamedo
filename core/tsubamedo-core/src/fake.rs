//! Made-up values for `{{$randomFirstName}}` and the rest of Postman's dynamic variables, so a
//! collection brought from Postman (or Insomnia's `faker` tag) sends what it sent there. Small
//! word lists, enough for test data: the app stays light.

use std::fmt::Write;

const FIRST: &[&str] = &["Anna", "Boris", "Clara", "Daniel", "Elena", "Felix", "Grace", "Hugo", "Iris", "Jonas", "Kira", "Leo", "Maya", "Nina", "Oscar", "Paula", "Rosa", "Sam", "Tina", "Victor"];
const LAST: &[&str] = &["Smith", "Johnson", "Brown", "Garcia", "Miller", "Davis", "Wilson", "Moore", "Taylor", "Clark", "Lewis", "Walker", "Young", "King", "Hill", "Green", "Baker", "Adams", "Nelson", "Carter"];
const PREFIX: &[&str] = &["Mr.", "Mrs.", "Ms.", "Miss", "Dr."];
const SUFFIX: &[&str] = &["Jr.", "Sr.", "I", "II", "III", "IV", "V", "MD", "DDS", "PhD", "DVM"];
const JOB_AREA: &[&str] = &["Solutions", "Program", "Brand", "Security", "Research", "Marketing", "Directives", "Implementation", "Integration", "Functionality", "Response", "Paradigm", "Tactics", "Identity", "Markets", "Group", "Applications", "Optimization", "Operations", "Infrastructure", "Data", "Quality", "Accounts", "Creative"];
const JOB_DESCRIPTOR: &[&str] = &["Lead", "Senior", "Direct", "Corporate", "Dynamic", "Future", "Product", "National", "Regional", "District", "Central", "Global", "Customer", "Investor", "Internal", "International", "Legacy", "Forward", "Chief", "Principal"];
const JOB_TYPE: &[&str] = &["Supervisor", "Associate", "Executive", "Liaison", "Officer", "Manager", "Engineer", "Specialist", "Director", "Coordinator", "Administrator", "Architect", "Analyst", "Designer", "Planner", "Orchestrator", "Technician", "Developer", "Producer", "Consultant", "Assistant", "Facilitator", "Agent", "Representative", "Strategist"];
const CITY: &[&str] = &["Springfield", "Riverside", "Fairview", "Madison", "Georgetown", "Clinton", "Franklin", "Greenville", "Bristol", "Salem", "Oakland", "Lakewood", "Ashland", "Milton", "Newport"];
const STREET: &[&str] = &["Main", "Oak", "Pine", "Maple", "Cedar", "Elm", "Washington", "Lake", "Hill", "Park", "Sunset", "River", "Church", "Mill", "Spring"];
const STREET_SUFFIX: &[&str] = &["Street", "Avenue", "Road", "Lane", "Drive", "Court", "Place", "Way", "Boulevard", "Terrace"];
const COUNTRY: &[(&str, &str)] = &[
    ("United States", "US"),
    ("Germany", "DE"),
    ("France", "FR"),
    ("Japan", "JP"),
    ("Brazil", "BR"),
    ("Canada", "CA"),
    ("Italy", "IT"),
    ("Spain", "ES"),
    ("Kazakhstan", "KZ"),
    ("Australia", "AU"),
    ("India", "IN"),
    ("Mexico", "MX"),
    ("Norway", "NO"),
    ("Serbia", "RS"),
    ("Georgia", "GE"),
];
const COLOR: &[&str] = &["red", "green", "blue", "yellow", "purple", "orange", "black", "white", "gray", "pink", "teal", "lime", "maroon", "navy", "olive", "silver", "gold", "indigo", "violet", "turquoise"];
const ABBREVIATION: &[&str] = &["ADP", "AGP", "AI", "API", "ASCII", "CSS", "EXE", "FTP", "GB", "HDD", "HEX", "HTTP", "IB", "IP", "JBOD", "JSON", "OCR", "PCI", "PNG", "RAM", "RSS", "SAS", "SCSI", "SDD", "SMS", "SMTP", "SQL", "SSD", "SSL", "TCP", "THX", "TLS", "UDP", "USB", "UTF8", "XML", "XSS"];
const LOCALE: &[&str] = &["en", "en_US", "en_GB", "ru", "de", "fr", "es", "it", "ja", "ko", "pt_BR", "zh_CN", "nl", "pl", "sv", "tr", "uk"];
const USER_AGENT: &[&str] = &[
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64; rv:125.0) Gecko/20100101 Firefox/125.0",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1",
];
const NOUN: &[&str] = &["driver", "protocol", "bandwidth", "panel", "microchip", "program", "port", "card", "array", "interface", "system", "sensor", "firewall", "hard drive", "pixel", "alarm", "feed", "monitor", "application", "transmitter", "bus", "circuit", "capacitor", "matrix"];
const VERB: &[&str] = &["back up", "bypass", "hack", "override", "compress", "copy", "navigate", "index", "connect", "generate", "quantify", "calculate", "synthesize", "input", "transmit", "program", "reboot", "parse"];
const INGVERB: &[&str] = &["backing up", "bypassing", "hacking", "overriding", "compressing", "copying", "navigating", "indexing", "connecting", "generating", "quantifying", "calculating", "synthesizing", "transmitting", "programming", "parsing"];
const ADJECTIVE: &[&str] = &["auxiliary", "primary", "back-end", "digital", "open-source", "virtual", "cross-platform", "redundant", "online", "haptic", "multi-byte", "bluetooth", "wireless", "1080p", "neural", "optical", "solid state", "mobile"];
const LOREM: &[&str] = &["lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit", "sed", "do", "eiusmod", "tempor", "incididunt", "ut", "labore", "et", "dolore", "magna", "aliqua", "enim", "ad", "minim", "veniam", "quis", "nostrud", "exercitation", "ullamco", "laboris", "nisi", "aliquip", "ex", "ea", "commodo", "consequat"];
const COMPANY_SUFFIX: &[&str] = &["Inc", "and Sons", "LLC", "Group"];
const BS_ADJECTIVE: &[&str] = &["clicks-and-mortar", "value-added", "vertical", "proactive", "robust", "revolutionary", "scalable", "leading-edge", "innovative", "intuitive", "strategic", "e-business", "mission-critical", "sticky", "one-to-one", "24/7", "end-to-end", "global", "B2B", "B2C", "granular", "frictionless", "virtual", "viral", "dynamic", "real-time", "seamless"];
const BS_BUZZ: &[&str] = &["synergize", "strategize", "empower", "embrace", "enable", "orchestrate", "leverage", "streamline", "cultivate", "facilitate", "integrate", "transform", "monetize", "harness", "optimize", "repurpose", "reinvent", "aggregate", "engage", "unleash", "evolve", "scale", "incubate", "deploy"];
const BS_NOUN: &[&str] = &["synergies", "web-readiness", "paradigms", "markets", "partnerships", "infrastructures", "platforms", "initiatives", "channels", "eyeballs", "communities", "solutions", "e-tailers", "action-items", "portals", "niches", "technologies", "content", "supply-chains", "convergence", "relationships", "architectures", "interfaces", "e-markets", "e-commerce", "systems", "bandwidth", "models", "mindshare", "deliverables", "users", "schemas", "networks", "applications", "metrics", "experiences"];
const CATCH_ADJECTIVE: &[&str] = &["Adaptive", "Advanced", "Ameliorated", "Assimilated", "Automated", "Balanced", "Business-focused", "Centralized", "Cloned", "Compatible", "Configurable", "Cross-group", "Customer-focused", "Customizable", "Decentralized", "De-engineered", "Devolved", "Digitized", "Distributed", "Diverse", "Enhanced", "Enterprise-wide", "Ergonomic", "Exclusive", "Expanded", "Extended", "Face to face", "Focused", "Front-line", "Fully-configurable", "Fundamental", "Future-proofed", "Grass-roots", "Horizontal", "Implemented", "Innovative", "Integrated", "Intuitive", "Inverse", "Managed", "Mandatory", "Monitored", "Multi-channelled", "Networked", "Open-architected", "Open-source", "Operative", "Optimized", "Optional", "Organic", "Organized", "Persevering", "Persistent", "Phased", "Polarised", "Pre-emptive", "Proactive", "Profit-focused", "Profound", "Programmable", "Progressive", "Public-key", "Quality-focused", "Reactive", "Realigned", "Re-contextualized", "Re-engineered", "Reduced", "Reverse-engineered", "Right-sized", "Robust", "Seamless", "Secured", "Self-enabling", "Sharable", "Stand-alone", "Streamlined", "Switchable", "Synchronised", "Synergistic", "Synergized", "Team-oriented", "Total", "Triple-buffered", "Universal", "Up-sized", "Upgradable", "User-centric", "User-friendly", "Versatile", "Virtual", "Visionary", "Vision-oriented"];
const CATCH_DESCRIPTOR: &[&str] = &["24 hour", "24/7", "3rd generation", "4th generation", "5th generation", "6th generation", "actuating", "analyzing", "asymmetric", "asynchronous", "attitude-oriented", "background", "bandwidth-monitored", "bi-directional", "bifurcated", "bottom-line", "clear-thinking", "client-driven", "client-server", "coherent", "cohesive", "composite", "context-sensitive", "contextually-based", "content-based", "dedicated", "demand-driven", "didactic", "directional", "discrete", "disintermediate", "dynamic", "eco-centric", "empowering", "encompassing", "even-keeled", "executive", "explicit", "exuding", "fault-tolerant", "foreground", "fresh-thinking", "full-range", "global", "grid-enabled", "heuristic", "high-level", "holistic", "homogeneous", "human-resource", "hybrid", "impactful", "incremental", "intangible", "interactive", "intermediate", "leading edge", "local", "logistical", "maximized", "methodical", "mission-critical", "mobile", "modular", "motivating", "multimedia", "multi-state", "multi-tasking", "national", "needs-based", "neutral", "next generation", "non-volatile", "object-oriented", "optimal", "optimizing", "radical", "real-time", "reciprocal", "regional", "responsive", "scalable", "secondary", "solution-oriented", "stable", "static", "systematic", "systemic", "system-worthy", "tangible", "tertiary", "transitional", "uniform", "upward-trending", "user-facing", "value-added", "web-enabled", "well-modulated", "zero administration", "zero defect", "zero tolerance"];
const CATCH_NOUN: &[&str] = &["ability", "access", "adapter", "algorithm", "alliance", "analyzer", "application", "approach", "architecture", "archive", "artificial intelligence", "array", "attitude", "benchmark", "budgetary management", "capability", "capacity", "challenge", "circuit", "collaboration", "complexity", "concept", "conglomeration", "contingency", "core", "customer loyalty", "database", "data-warehouse", "definition", "emulation", "encoding", "encryption", "extranet", "firmware", "flexibility", "focus group", "forecast", "frame", "framework", "function", "functionalities", "Graphic Interface", "groupware", "Graphical User Interface", "hardware", "help-desk", "hierarchy", "hub", "implementation", "info-mediaries", "infrastructure", "initiative", "installation", "instruction set", "interface", "internet solution", "intranet", "knowledge user", "knowledge base", "local area network", "leverage", "matrices", "matrix", "methodology", "middleware", "migration", "model", "moderator", "monitoring", "moratorium", "neural-net", "open architecture", "open system", "orchestration", "paradigm", "parallelism", "policy", "portal", "pricing structure", "process improvement", "product", "productivity", "project", "projection", "protocol", "secured line", "service-desk", "software", "solution", "standardization", "strategy", "structure", "success", "superstructure", "support", "synergy", "system engine", "task-force", "throughput", "time-frame", "toolset", "utilisation", "website", "workforce"];
const DB_COLUMN: &[&str] = &["id", "title", "name", "email", "phone", "token", "group", "category", "password", "comment", "avatar", "status", "createdAt", "updatedAt"];
const DB_TYPE: &[&str] = &["int", "varchar", "text", "date", "datetime", "tinyint", "time", "timestamp", "smallint", "mediumint", "bigint", "decimal", "float", "double", "real", "bit", "boolean", "serial", "blob", "binary", "enum", "set", "geometry", "point"];
const DB_COLLATION: &[&str] = &["utf8_unicode_ci", "utf8_general_ci", "utf8_bin", "ascii_bin", "ascii_general_ci", "cp1250_bin", "cp1250_general_ci"];
const DB_ENGINE: &[&str] = &["InnoDB", "MyISAM", "MEMORY", "CSV", "BLACKHOLE", "ARCHIVE"];
const WEEKDAY: &[&str] = &["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTH: &[&str] = &["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DOMAIN_SUFFIX: &[&str] = &["com", "net", "org", "info", "biz", "name", "io", "dev"];
const FILE: &[(&str, &str, &str)] = &[
    ("pdf", "application", "application/pdf"),
    ("json", "application", "application/json"),
    ("zip", "application", "application/zip"),
    ("png", "image", "image/png"),
    ("jpeg", "image", "image/jpeg"),
    ("gif", "image", "image/gif"),
    ("txt", "text", "text/plain"),
    ("csv", "text", "text/csv"),
    ("html", "text", "text/html"),
    ("mp3", "audio", "audio/mpeg"),
    ("wav", "audio", "audio/wav"),
    ("mp4", "video", "video/mp4"),
    ("mpeg", "video", "video/mpeg"),
];
const DIRECTORY: &[&str] = &["/usr", "/usr/lib", "/usr/local/bin", "/var/log", "/var/lib", "/etc", "/opt", "/home/user", "/tmp", "/srv", "/boot", "/lib", "/mnt", "/dev", "/proc", "/sys"];
const TRANSACTION: &[&str] = &["deposit", "withdrawal", "payment", "invoice"];
const CURRENCY: &[(&str, &str, &str)] = &[
    ("USD", "US Dollar", "$"),
    ("EUR", "Euro", "€"),
    ("GBP", "Pound Sterling", "£"),
    ("JPY", "Yen", "¥"),
    ("CNY", "Yuan Renminbi", "¥"),
    ("CHF", "Swiss Franc", "CHF"),
    ("INR", "Indian Rupee", "₹"),
    ("KZT", "Tenge", "₸"),
    ("BRL", "Brazilian Real", "R$"),
];
const PRODUCT: &[&str] = &["Chair", "Car", "Computer", "Keyboard", "Mouse", "Bike", "Ball", "Gloves", "Pants", "Shirt", "Table", "Shoes", "Hat", "Towels", "Soap", "Tuna", "Chicken", "Fish", "Cheese", "Bacon", "Pizza", "Salad", "Sausages", "Chips"];
const PRODUCT_ADJECTIVE: &[&str] = &["Small", "Ergonomic", "Rustic", "Intelligent", "Gorgeous", "Incredible", "Fantastic", "Practical", "Sleek", "Awesome", "Generic", "Handcrafted", "Handmade", "Licensed", "Refined", "Unbranded", "Tasty"];
const PRODUCT_MATERIAL: &[&str] = &["Steel", "Wooden", "Concrete", "Plastic", "Cotton", "Granite", "Rubber", "Metal", "Soft", "Fresh", "Frozen"];
const DEPARTMENT: &[&str] = &["Books", "Movies", "Music", "Games", "Electronics", "Computers", "Home", "Garden", "Tools", "Grocery", "Health", "Beauty", "Toys", "Kids", "Baby", "Clothing", "Shoes", "Jewelery", "Sports", "Outdoors", "Automotive", "Industrial"];
const IMAGE_TOPICS: &[(&str, &str)] = &[
    ("Abstract", "abstract"),
    ("Animals", "animals"),
    ("Business", "business"),
    ("Cats", "cats"),
    ("City", "city"),
    ("Food", "food"),
    ("Nightlife", "nightlife"),
    ("Fashion", "fashion"),
    ("People", "people"),
    ("Nature", "nature"),
    ("Sports", "sports"),
    ("Transport", "transport"),
];

/// Postman's dynamic variables (learning.postman.com, «Dynamic variables»), every one of which
/// [`value`] knows.
pub const NAMES: &[&str] = &[
    "guid", "randomUUID", "randomAlphaNumeric", "randomBoolean", "randomColor", "randomHexColor", "randomAbbreviation", "randomIP", "randomIPV6",
    "randomMACAddress", "randomPassword", "randomLocale", "randomUserAgent", "randomProtocol", "randomSemver", "randomFirstName", "randomLastName",
    "randomFullName", "randomNamePrefix", "randomNameSuffix", "randomJobArea", "randomJobDescriptor", "randomJobTitle", "randomJobType",
    "randomPhoneNumber", "randomPhoneNumberExt", "randomCity", "randomStreetName", "randomStreetAddress", "randomCountry", "randomCountryCode",
    "randomLatitude", "randomLongitude", "randomAvatarImage", "randomImageUrl", "randomAbstractImage", "randomAnimalsImage", "randomBusinessImage",
    "randomCatsImage", "randomCityImage", "randomFoodImage", "randomNightlifeImage", "randomFashionImage", "randomPeopleImage", "randomNatureImage",
    "randomSportsImage", "randomTransportImage", "randomImageDataUri", "randomBankAccount", "randomBankAccountName", "randomCreditCardMask",
    "randomBankAccountBic", "randomBankAccountIban", "randomTransactionType", "randomCurrencyCode", "randomCurrencyName", "randomCurrencySymbol",
    "randomBitcoin", "randomCompanyName", "randomCompanySuffix", "randomBs", "randomBsAdjective", "randomBsBuzz", "randomBsNoun", "randomCatchPhrase",
    "randomCatchPhraseAdjective", "randomCatchPhraseDescriptor", "randomCatchPhraseNoun", "randomDatabaseColumn", "randomDatabaseType",
    "randomDatabaseCollation", "randomDatabaseEngine", "randomDateFuture", "randomDatePast", "randomDateRecent", "randomWeekday", "randomMonth",
    "randomDomainName", "randomDomainSuffix", "randomDomainWord", "randomEmail", "randomExampleEmail", "randomUserName", "randomUrl", "randomFileName",
    "randomFileType", "randomFileExt", "randomCommonFileName", "randomCommonFileType", "randomCommonFileExt", "randomFilePath", "randomDirectoryPath",
    "randomMimeType", "randomPrice", "randomProduct", "randomProductAdjective", "randomProductMaterial", "randomProductName", "randomDepartment",
    "randomNoun", "randomVerb", "randomIngverb", "randomAdjective", "randomWord", "randomWords", "randomPhrase", "randomLoremWord", "randomLoremWords",
    "randomLoremSentence", "randomLoremSentences", "randomLoremParagraph", "randomLoremParagraphs", "randomLoremText", "randomLoremSlug",
    "randomLoremLines",
];

fn pick<T: Copy>(list: &[T]) -> T {
    list[fastrand::usize(..list.len())]
}

fn digits(n: usize) -> String {
    (0..n).map(|_| char::from(b'0' + fastrand::u8(0..10))).collect()
}

fn alnum(n: usize) -> String {
    (0..n).map(|_| fastrand::alphanumeric()).collect()
}

fn hex(n: usize) -> String {
    (0..n).map(|_| char::from_digit(fastrand::u32(0..16), 16).unwrap()).collect()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn lorem_words(n: usize) -> String {
    (0..n).map(|_| pick(LOREM)).collect::<Vec<_>>().join(" ")
}

fn sentence() -> String {
    format!("{}.", capitalize(&lorem_words(fastrand::usize(4..10))))
}

fn sentences(n: usize) -> String {
    (0..n).map(|_| sentence()).collect::<Vec<_>>().join(" ")
}

fn paragraph() -> String {
    sentences(3)
}

fn word() -> String {
    match fastrand::u8(0..4) {
        0 => pick(NOUN),
        1 => pick(VERB),
        2 => pick(ADJECTIVE),
        _ => pick(INGVERB),
    }
    .to_string()
}

fn domain_word() -> String {
    format!("{}-{}", pick(FIRST), pick(LAST)).to_lowercase()
}

/// A moment around now, `days` away at most (negative for the past), as an ISO 8601 date.
fn date(from_days: i64, to_days: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let at = now + fastrand::i64(from_days * 86_400..to_days * 86_400);
    crate::template::iso(std::time::Duration::from_secs(at.max(0) as u64))
}

fn phrase() -> String {
    format!(
        "If we {} the {}, we can get to the {} {} through the {} {} {}!",
        pick(VERB),
        pick(NOUN),
        pick(ABBREVIATION),
        pick(NOUN),
        pick(ADJECTIVE),
        pick(ABBREVIATION),
        pick(NOUN)
    )
}

/// The value of `{{$<name>}}`, for the names Postman knows; `None` for any other.
pub fn value(name: &str) -> Option<String> {
    let first = || pick(FIRST);
    let last = || pick(LAST);
    let file = || pick(FILE);
    Some(match name {
        "guid" | "randomUUID" => uuid::Uuid::new_v4().to_string(),
        "randomAlphaNumeric" => alnum(1).to_lowercase(),
        "randomBoolean" => fastrand::bool().to_string(),
        "randomColor" => pick(COLOR).into(),
        "randomHexColor" => format!("#{}", hex(6)),
        "randomAbbreviation" => pick(ABBREVIATION).into(),
        "randomIP" => format!("{}.{}.{}.{}", fastrand::u8(1..255), fastrand::u8(..), fastrand::u8(..), fastrand::u8(1..255)),
        "randomIPV6" => (0..8).map(|_| hex(4)).collect::<Vec<_>>().join(":"),
        "randomMACAddress" => (0..6).map(|_| hex(2)).collect::<Vec<_>>().join(":"),
        "randomPassword" => alnum(15),
        "randomLocale" => pick(LOCALE).into(),
        "randomUserAgent" => pick(USER_AGENT).into(),
        "randomProtocol" => pick(&["http", "https"]).into(),
        "randomSemver" => format!("{}.{}.{}", fastrand::u8(0..10), fastrand::u8(0..10), fastrand::u8(0..10)),
        "randomFirstName" => first().into(),
        "randomLastName" => last().into(),
        "randomFullName" => format!("{} {}", first(), last()),
        "randomNamePrefix" => pick(PREFIX).into(),
        "randomNameSuffix" => pick(SUFFIX).into(),
        "randomJobArea" => pick(JOB_AREA).into(),
        "randomJobDescriptor" => pick(JOB_DESCRIPTOR).into(),
        "randomJobTitle" => format!("{} {} {}", pick(JOB_DESCRIPTOR), pick(JOB_AREA), pick(JOB_TYPE)),
        "randomJobType" => pick(JOB_TYPE).into(),
        "randomPhoneNumber" => format!("{}-{}-{}", digits(3), digits(3), digits(4)),
        "randomPhoneNumberExt" => format!("{}-{}-{}-{}", digits(2), digits(3), digits(3), digits(4)),
        "randomCity" => pick(CITY).into(),
        "randomStreetName" => format!("{} {}", pick(STREET), pick(STREET_SUFFIX)),
        "randomStreetAddress" => format!("{} {} {}", fastrand::u16(1..10_000), pick(STREET), pick(STREET_SUFFIX)),
        "randomCountry" => pick(COUNTRY).0.into(),
        "randomCountryCode" => pick(COUNTRY).1.into(),
        "randomLatitude" => format!("{:.4}", fastrand::f64() * 180.0 - 90.0),
        "randomLongitude" => format!("{:.4}", fastrand::f64() * 360.0 - 180.0),
        "randomAvatarImage" => format!("https://i.pravatar.cc/128?img={}", fastrand::u8(1..70)),
        "randomImageUrl" => format!("https://picsum.photos/640/480?random={}", fastrand::u32(..)),
        "randomImageDataUri" => {
            "data:image/svg+xml;charset=UTF-8,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%22640%22%20height%3D%22480%22%3E%3Crect%20width%3D%22100%25%22%20height%3D%22100%25%22%20fill%3D%22grey%22%2F%3E%3C%2Fsvg%3E".into()
        }
        "randomBankAccount" => digits(8),
        "randomBankAccountName" => format!("{} Account", pick(&["Checking", "Savings", "Money Market", "Investment", "Home Loan", "Credit Card", "Auto Loan", "Personal Loan"])),
        "randomCreditCardMask" => digits(4),
        "randomBankAccountBic" => format!("{}{}{}", (0..4).map(|_| fastrand::uppercase()).collect::<String>(), pick(COUNTRY).1, alnum(2).to_uppercase()),
        "randomBankAccountIban" => format!("{}{}{}", pick(&["DE", "FR", "GB", "NL", "ES"]), digits(2), digits(18)),
        "randomTransactionType" => pick(TRANSACTION).into(),
        "randomCurrencyCode" => pick(CURRENCY).0.into(),
        "randomCurrencyName" => pick(CURRENCY).1.into(),
        "randomCurrencySymbol" => pick(CURRENCY).2.into(),
        "randomBitcoin" => {
            const BASE58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
            let tail: String = (0..fastrand::usize(25..33)).map(|_| char::from(BASE58[fastrand::usize(..BASE58.len())])).collect();
            format!("{}{tail}", pick(&["1", "3"]))
        }
        "randomCompanyName" => format!("{} {}", last(), pick(COMPANY_SUFFIX)),
        "randomCompanySuffix" => pick(COMPANY_SUFFIX).into(),
        "randomBs" => format!("{} {} {}", pick(BS_BUZZ), pick(BS_ADJECTIVE), pick(BS_NOUN)),
        "randomBsAdjective" => pick(BS_ADJECTIVE).into(),
        "randomBsBuzz" => pick(BS_BUZZ).into(),
        "randomBsNoun" => pick(BS_NOUN).into(),
        "randomCatchPhrase" => format!("{} {} {}", pick(CATCH_ADJECTIVE), pick(CATCH_DESCRIPTOR), pick(CATCH_NOUN)),
        "randomCatchPhraseAdjective" => pick(CATCH_ADJECTIVE).into(),
        "randomCatchPhraseDescriptor" => pick(CATCH_DESCRIPTOR).into(),
        "randomCatchPhraseNoun" => pick(CATCH_NOUN).into(),
        "randomDatabaseColumn" => pick(DB_COLUMN).into(),
        "randomDatabaseType" => pick(DB_TYPE).into(),
        "randomDatabaseCollation" => pick(DB_COLLATION).into(),
        "randomDatabaseEngine" => pick(DB_ENGINE).into(),
        "randomDateFuture" => date(1, 365),
        "randomDatePast" => date(-365, -1),
        "randomDateRecent" => date(-1, 0),
        "randomWeekday" => pick(WEEKDAY).into(),
        "randomMonth" => pick(MONTH).into(),
        "randomDomainName" => format!("{}.{}", domain_word(), pick(DOMAIN_SUFFIX)),
        "randomDomainSuffix" => pick(DOMAIN_SUFFIX).into(),
        "randomDomainWord" => domain_word(),
        "randomEmail" => format!("{}.{}@{}.{}", first(), last(), pick(&["gmail", "yahoo", "hotmail", "mail"]), pick(&["com", "org", "net"])),
        "randomExampleEmail" => format!("{}.{}@example.{}", first(), last(), pick(&["com", "org", "net"])),
        "randomUserName" => format!("{}.{}{}", first(), last(), fastrand::u8(..100)),
        "randomUrl" => format!("https://{}.{}", domain_word(), pick(DOMAIN_SUFFIX)),
        "randomFileName" | "randomCommonFileName" => format!("{}.{}", lorem_words(2).replace(' ', "_"), file().0),
        "randomFileType" | "randomCommonFileType" => file().1.into(),
        "randomFileExt" | "randomCommonFileExt" => file().0.into(),
        "randomFilePath" => format!("{}/{}.{}", pick(DIRECTORY), pick(LOREM), file().0),
        "randomDirectoryPath" => pick(DIRECTORY).into(),
        "randomMimeType" => file().2.into(),
        "randomPrice" => format!("{}.{}", fastrand::u16(1..1000), digits(2)),
        "randomProduct" => pick(PRODUCT).into(),
        "randomProductAdjective" => pick(PRODUCT_ADJECTIVE).into(),
        "randomProductMaterial" => pick(PRODUCT_MATERIAL).into(),
        "randomProductName" => format!("{} {} {}", pick(PRODUCT_ADJECTIVE), pick(PRODUCT_MATERIAL), pick(PRODUCT)),
        "randomDepartment" => pick(DEPARTMENT).into(),
        "randomNoun" => pick(NOUN).into(),
        "randomVerb" => pick(VERB).into(),
        "randomIngverb" => pick(INGVERB).into(),
        "randomAdjective" => pick(ADJECTIVE).into(),
        "randomWord" => word(),
        "randomWords" => (0..fastrand::usize(2..5)).map(|_| word()).collect::<Vec<_>>().join(" "),
        "randomPhrase" => phrase(),
        "randomLoremWord" => pick(LOREM).into(),
        "randomLoremWords" => lorem_words(3),
        "randomLoremSentence" => sentence(),
        "randomLoremSentences" => sentences(fastrand::usize(2..6)),
        "randomLoremParagraph" | "randomLoremText" => paragraph(),
        "randomLoremParagraphs" => (0..3).map(|_| paragraph()).collect::<Vec<_>>().join("\n \r"),
        "randomLoremSlug" => lorem_words(3).replace(' ', "-"),
        "randomLoremLines" => (0..fastrand::usize(1..5)).map(|_| sentence()).collect::<Vec<_>>().join("\n"),
        _ => {
            // The image of a topic: `$randomCatsImage`, `$randomNatureImage`…
            let topic = name.strip_prefix("random")?.strip_suffix("Image")?;
            let (_, slug) = IMAGE_TOPICS.iter().find(|(t, _)| *t == topic)?;
            let mut url = String::new();
            let _ = write!(url, "https://picsum.photos/seed/{slug}{}/640/480", fastrand::u32(..));
            url
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn every_postman_variable_has_a_value() {
        for name in NAMES {
            for _ in 0..20 {
                let v = value(name).unwrap_or_else(|| panic!("нет ${name}"));
                assert!(!v.is_empty(), "${name}");
            }
        }
        assert_eq!(value("randomNope"), None);
        assert_eq!(value("randomNopeImage"), None);
    }

    #[test]
    fn shapes() {
        assert_eq!(value("randomUUID").unwrap().len(), 36);
        assert!(value("randomEmail").unwrap().contains('@'));
        assert!(value("randomHexColor").unwrap().starts_with('#'));
        assert_eq!(value("randomIPV6").unwrap().split(':').count(), 8);
        assert!(value("randomDateFuture").unwrap() > value("randomDatePast").unwrap());
        assert!(value("randomLoremSlug").unwrap().split('-').count() == 3);
        assert!(["true", "false"].contains(&value("randomBoolean").unwrap().as_str()));
    }
}
