use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::Manager;

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);
static BUILTIN_PAST_CACHE: OnceLock<Mutex<HashMap<String, ProblemRecord>>> = OnceLock::new();
const DATASET_POLICY_VERSION: &str = "dataset-policy-v4-generator-quality";
const GENERATION_MAX_TOKENS: u32 = 32_000;
const TEST_CASE_SEPARATOR: &str = "---AUTO_JUDGE_CASE---";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Topic {
    id: String,
    title: String,
    source: String,
    year: Option<String>,
    path: String,
    excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    id: String,
    title: String,
    created_at: String,
    difficulty: String,
    topic_titles: Vec<String>,
    test_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    api_key: String,
    api_url: String,
    model: String,
    use_cache: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapData {
    topics: Vec<Topic>,
    past_problems: Vec<HistoryEntry>,
    history: Vec<HistoryEntry>,
    data_directory: String,
    settings: AppSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateRequest {
    api_key: String,
    api_url: String,
    model: String,
    use_cache: bool,
    topics: Vec<Topic>,
    difficulty: String,
    include_file_io: bool,
    extra_requirements: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IoMode {
    kind: String,
    input_file: Option<String>,
    output_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceSolution {
    language: String,
    code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DataGenerator {
    language: String,
    code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SampleCase {
    input: String,
    output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeneratedProblemDraft {
    title: String,
    difficulty: String,
    statement: String,
    input_format: String,
    output_format: String,
    constraints: Vec<String>,
    tags: Vec<String>,
    io_mode: IoMode,
    samples: Vec<SampleCase>,
    reference_solution: ReferenceSolution,
    #[serde(default)]
    data_generator: Option<DataGenerator>,
    #[serde(default)]
    test_inputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestFile {
    name: String,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestCase {
    name: String,
    input: String,
    expected_output: String,
    #[serde(default)]
    files: Vec<TestFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProblemRecord {
    id: String,
    created_at: String,
    title: String,
    difficulty: String,
    statement: String,
    input_format: String,
    output_format: String,
    constraints: Vec<String>,
    tags: Vec<String>,
    topic_titles: Vec<String>,
    io_mode: IoMode,
    samples: Vec<TestCase>,
    tests: Vec<TestCase>,
    reference_solution: ReferenceSolution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JudgeRequest {
    problem_id: String,
    language: String,
    code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaseResult {
    name: String,
    status: String,
    elapsed_ms: u128,
    expected_output: String,
    actual_output: String,
    stderr: String,
    artifact_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JudgeResult {
    status: String,
    passed: usize,
    total: usize,
    compile_elapsed_ms: u128,
    run_elapsed_ms: u128,
    compile_stdout: String,
    compile_stderr: String,
    cases: Vec<CaseResult>,
}

#[derive(Debug, Clone)]
struct CompiledProgram {
    work_dir: PathBuf,
    binary_path: PathBuf,
}

#[derive(Debug, Clone)]
struct RunOutput {
    status: String,
    stdout: String,
    stderr: String,
    elapsed_ms: u128,
}

fn now_millis() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| format!("系统时间异常：{error}"))
}

fn now_string() -> Result<String, String> {
    Ok(now_millis()?.to_string())
}

fn app_data_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("data"))
        .map_err(|error| format!("无法解析应用数据目录：{error}"))
}

fn history_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_directory(app)?.join("history"))
}

fn cache_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_directory(app)?.join("agent-cache"))
}

fn builtin_problem_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_directory(app)?.join("builtin-past-problems"))
}

fn judge_runs_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_directory(app)?.join("judge-runs"))
}

fn case_files_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_directory(app)?.join("case-files"))
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_directory(app)?.join("settings.json"))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建目录：{error}"))?;
    }
    let temporary = path.with_extension("json.tmp");
    let text =
        serde_json::to_string_pretty(value).map_err(|error| format!("无法序列化数据：{error}"))?;
    fs::write(&temporary, text).map_err(|error| format!("无法写入临时文件：{error}"))?;
    fs::rename(&temporary, path).map_err(|error| format!("无法保存数据文件：{error}"))?;
    Ok(())
}

fn safe_file_stem(value: &str) -> String {
    let stem = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let stem = stem.trim_matches('.').trim_matches('_');
    if stem.is_empty() {
        "case".to_string()
    } else {
        stem.to_string()
    }
}

fn write_case_artifact(
    run_dir: &Path,
    case: &TestCase,
    status: &str,
    elapsed_ms: u128,
    actual_output: &str,
    stderr: &str,
) -> Result<PathBuf, String> {
    let case_dir = run_dir.join(safe_file_stem(&case.name));
    fs::create_dir_all(&case_dir).map_err(|error| format!("无法创建评测样例目录：{error}"))?;
    fs::write(case_dir.join("input.txt"), &case.input)
        .map_err(|error| format!("无法写入评测输入：{error}"))?;
    fs::write(case_dir.join("expected.txt"), &case.expected_output)
        .map_err(|error| format!("无法写入标准输出：{error}"))?;
    fs::write(case_dir.join("actual.txt"), actual_output)
        .map_err(|error| format!("无法写入用户输出：{error}"))?;
    fs::write(case_dir.join("stderr.txt"), stderr)
        .map_err(|error| format!("无法写入错误输出：{error}"))?;
    for file in &case.files {
        write_support_file(&case_dir, file)?;
    }
    fs::write(
        case_dir.join("summary.txt"),
        format!(
            "case: {}\nstatus: {}\nelapsed_ms: {}\n",
            case.name, status, elapsed_ms
        ),
    )
    .map_err(|error| format!("无法写入评测摘要：{error}"))?;
    Ok(case_dir)
}

fn support_file_name(file_name: &str) -> Result<String, String> {
    Path::new(file_name)
        .file_name()
        .map(|name| safe_file_stem(&name.to_string_lossy()))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("非法文件名：{file_name}"))
}

fn write_support_file(parent: &Path, file: &TestFile) -> Result<PathBuf, String> {
    let path = parent.join(support_file_name(&file.name)?);
    fs::write(&path, &file.content)
        .map_err(|error| format!("无法写入附件文件 {}：{error}", file.name))?;
    Ok(path)
}

fn open_filesystem_path(path: &Path) -> Result<(), String> {
    let mut command = if cfg!(target_os = "windows") {
        let mut command = Command::new("explorer");
        command.arg(path);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(path);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开本地文件：{error}"))
}

fn default_settings() -> AppSettings {
    AppSettings {
        api_key: String::new(),
        api_url: String::new(),
        model: "deepseek-v4-pro".to_string(),
        use_cache: true,
    }
}

fn read_settings(app: &tauri::AppHandle) -> Result<AppSettings, String> {
    let path = settings_path(app)?;
    match fs::read_to_string(path) {
        Ok(content) => {
            serde_json::from_str(&content).map_err(|error| format!("设置文件格式错误：{error}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let settings = default_settings();
            write_json(&settings_path(app)?, &json!(settings))?;
            Ok(settings)
        }
        Err(error) => Err(format!("无法读取设置文件：{error}")),
    }
}

fn save_settings_file(app: &tauri::AppHandle, settings: &AppSettings) -> Result<(), String> {
    write_json(&settings_path(app)?, &json!(settings))
}

fn read_history_index(app: &tauri::AppHandle) -> Result<Vec<HistoryEntry>, String> {
    let path = history_directory(app)?.join("index.json");
    match fs::read_to_string(path) {
        Ok(content) => {
            serde_json::from_str(&content).map_err(|error| format!("历史索引格式错误：{error}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("无法读取历史索引：{error}")),
    }
}

fn hydrate_request_from_settings(
    app: &tauri::AppHandle,
    request: GenerateRequest,
) -> Result<GenerateRequest, String> {
    let settings = read_settings(app)?;
    Ok(GenerateRequest {
        api_key: if request.api_key.trim().is_empty() {
            settings.api_key
        } else {
            request.api_key
        },
        api_url: if request.api_url.trim().is_empty() {
            settings.api_url
        } else {
            request.api_url
        },
        model: if request.model.trim().is_empty() {
            settings.model
        } else {
            request.model
        },
        use_cache: request.use_cache,
        topics: request.topics,
        difficulty: if request.difficulty.trim().is_empty() {
            "medium".to_string()
        } else {
            request.difficulty
        },
        include_file_io: request.include_file_io,
        extra_requirements: request.extra_requirements,
    })
}

fn save_history_index(app: &tauri::AppHandle, history: &[HistoryEntry]) -> Result<(), String> {
    write_json(&history_directory(app)?.join("index.json"), &json!(history))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

fn courseware_directory() -> Option<PathBuf> {
    home_dir().map(|home| {
        home.join("Desktop")
            .join("Coding")
            .join("DS_helper")
            .join("resources")
    })
}

fn ds_lz_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ds_lz")
}

struct KnowledgeSpec {
    id: &'static str,
    title: &'static str,
    keywords: &'static [&'static str],
    excerpt: &'static str,
}

fn builtin_topic_prompt(spec: &KnowledgeSpec) -> String {
    format!(
        "内置命题提示：围绕「{}」出题。{} 题目应贴近数据结构机试风格，弱化竞赛式巧思，强化输入解析、结构体建模、状态维护、排序/遍历/模拟过程和边界处理。",
        spec.title, spec.excerpt
    )
}

fn knowledge_specs() -> Vec<KnowledgeSpec> {
    vec![
        KnowledgeSpec {
            id: "c-foundation",
            title: "C 语言数据表达：结构体、指针、数组与文件",
            keywords: &["复杂数据类型", "指针", "数组", "函数", "结构", "文件", "联合"],
            excerpt: "结构体建模、指针与数组访问、函数拆分、文件读写，可生成偏工程输入解析类题目。",
        },
        KnowledgeSpec {
            id: "sequence-list",
            title: "顺序表：查找、插入、删除与有序维护",
            keywords: &["线性表", "顺序表"],
            excerpt: "数组实现线性表，按位置/关键字查找，插入删除后的元素移动和边界处理。",
        },
        KnowledgeSpec {
            id: "linked-list",
            title: "单链表：建立、遍历、插入、删除与合并",
            keywords: &["单链表", "自引用结构"],
            excerpt: "指针结点、头指针、尾插/头插、按条件删除、链表合并与稳定输出。",
        },
        KnowledgeSpec {
            id: "advanced-list",
            title: "循环链表与双向链表",
            keywords: &["循环链表", "双向链表"],
            excerpt: "循环边界、前驱后继维护、双向指针一致性，适合模拟约瑟夫环或编辑序列。",
        },
        KnowledgeSpec {
            id: "string-array-glist",
            title: "串、数组与广义表基础",
            keywords: &["串", "数组", "广义表"],
            excerpt: "字符串扫描、模式处理、多维数组映射和递归式表结构分析。",
        },
        KnowledgeSpec {
            id: "stack",
            title: "栈：括号匹配、进出栈序列与表达式求值",
            keywords: &["栈", "后缀表达式", "表达式"],
            excerpt: "LIFO 过程模拟、合法出栈判断、中缀/后缀表达式转换与求值。",
        },
        KnowledgeSpec {
            id: "queue",
            title: "队列与循环队列",
            keywords: &["队", "循环队列"],
            excerpt: "FIFO 过程模拟，front/rear 更新，环形数组容量、空满判断与批量操作。",
        },
        KnowledgeSpec {
            id: "binary-tree-traversal",
            title: "二叉树遍历：递归、非递归与层序",
            keywords: &["树与二叉树", "二叉树遍历", "树的遍历", "递归和非递归"],
            excerpt: "前中后序、层序遍历，递归/栈/队列实现，以及由遍历序列还原结构。",
        },
        KnowledgeSpec {
            id: "bst-avl",
            title: "二叉排序树与平衡二叉树",
            keywords: &["BST", "二叉排序树", "平衡二叉树"],
            excerpt: "查找、插入、删除、旋转与树高维护，覆盖退化树和重复关键字策略。",
        },
        KnowledgeSpec {
            id: "huffman",
            title: "哈夫曼树、编码与 WPL",
            keywords: &["哈夫曼"],
            excerpt: "最小权值合并、带权路径长度、编码长度统计与优先队列实现。",
        },
        KnowledgeSpec {
            id: "heap",
            title: "堆与堆排序",
            keywords: &["堆"],
            excerpt: "大顶堆/小顶堆调整、建堆、优先级处理和堆排序过程输出。",
        },
        KnowledgeSpec {
            id: "sorting",
            title: "排序：冒泡、插入、选择、快速、归并、希尔与堆排序",
            keywords: &["排序", "冒泡", "插入", "选择", "快速", "归并", "希尔", "堆"],
            excerpt: "常见排序算法的过程模拟、关键字比较/交换统计、稳定性判断、按指定规则输出排序阶段结果。",
        },
        KnowledgeSpec {
            id: "multi-tree",
            title: "多叉树、森林与二叉树转换",
            keywords: &["多叉树"],
            excerpt: "孩子兄弟表示法，多叉树与森林遍历，结构转换后的序列输出。",
        },
        KnowledgeSpec {
            id: "graph-storage-traversal",
            title: "图存储与遍历：邻接矩阵、邻接表、DFS/BFS",
            keywords: &["图及其遍历", "图的遍历"],
            excerpt: "图的邻接矩阵/邻接表建模，深搜广搜，连通分量和访问顺序控制。",
        },
        KnowledgeSpec {
            id: "mst",
            title: "最小生成树：Prim 与 Kruskal",
            keywords: &["最小生成树"],
            excerpt: "无向连通带权图，Prim/Kruskal 选边过程，总权值与边集输出。",
        },
        KnowledgeSpec {
            id: "shortest-path",
            title: "最短路径：Dijkstra 与 Floyd",
            keywords: &["最短路径"],
            excerpt: "单源最短路径、多源最短路径、路径恢复、不可达点和权值边界。",
        },
        KnowledgeSpec {
            id: "search",
            title: "查找：顺序查找、折半查找、索引与判定树",
            keywords: &["查找", "索引"],
            excerpt: "有序表折半过程、查找比较次数、索引查找和判定树形态。",
        },
        KnowledgeSpec {
            id: "hash",
            title: "散列表与冲突处理",
            keywords: &["查找", "索引"],
            excerpt: "散列函数、链地址法、开放定址、冲突统计和装填因子。",
        },
        KnowledgeSpec {
            id: "btree",
            title: "B 树基础",
            keywords: &["B树"],
            excerpt: "多路平衡查找树，阶、关键字数量约束、插入分裂与查找路径。",
        },
    ]
}

fn collect_pptx_files_in(directory: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
            if extension != "pptx" {
                return None;
            }
            let file_name = path.file_name()?.to_string_lossy().to_string();
            Some((file_name, path))
        })
        .collect()
}

fn collect_courseware_files() -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    if let Some(directory) = courseware_directory() {
        files.extend(collect_pptx_files_in(&directory));
    }
    files.extend(collect_pptx_files_in(&ds_lz_directory()));
    files
}

fn collect_courseware_topics() -> Vec<Topic> {
    let files = collect_courseware_files();
    let mut topics = knowledge_specs()
        .into_iter()
        .map(|spec| {
            let prompt = builtin_topic_prompt(&spec);
            let matched = files
                .iter()
                .filter(|(file_name, _)| {
                    spec.keywords
                        .iter()
                        .any(|keyword| file_name.contains(keyword))
                })
                .collect::<Vec<_>>();
            if matched.is_empty() {
                return Topic {
                    id: format!("knowledge-{}", spec.id),
                    title: spec.title.to_string(),
                    source: "knowledge".to_string(),
                    year: None,
                    path: "内置知识点".to_string(),
                    excerpt: prompt,
                };
            }
            let source_files = matched
                .iter()
                .map(|(file_name, _)| file_name.as_str())
                .collect::<Vec<_>>()
                .join("；");
            let paths = matched
                .iter()
                .map(|(_, path)| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            Topic {
                id: format!("knowledge-{}", spec.id),
                title: spec.title.to_string(),
                source: "knowledge".to_string(),
                year: None,
                path: paths,
                excerpt: format!("{prompt} 参考课件：{source_files}"),
            }
        })
        .collect::<Vec<_>>();
    topics.sort_by(|first, second| first.title.cmp(&second.title));
    topics
}

fn collect_topics(_app: &tauri::AppHandle) -> Vec<Topic> {
    collect_courseware_topics()
}

fn validate_generation_request(request: &GenerateRequest) -> Result<(), String> {
    if request.api_key.trim().is_empty() {
        return Err("请先输入 API Key".to_string());
    }
    if request.api_url.trim().is_empty() {
        return Err("请先输入 API URL".to_string());
    }
    if !request.api_url.starts_with("https://") && !request.api_url.starts_with("http://") {
        return Err("API URL 必须以 http:// 或 https:// 开头".to_string());
    }
    if request.model.trim().is_empty() {
        return Err("请先输入模型名称".to_string());
    }
    if request.topics.is_empty() {
        return Err("请至少选择一个考点".to_string());
    }
    if !matches!(request.difficulty.as_str(), "easy" | "medium" | "hard") {
        return Err("题目难度必须是 easy、medium 或 hard".to_string());
    }
    Ok(())
}

fn difficulty_prompt_label(value: &str) -> &'static str {
    match value {
        "easy" => "简单：数据规模较小，重点考察按题意建模、输入输出细节、结构体数组和基础排序，代码量不少但分支较直观。",
        "hard" => "困难：题面包含多条业务规则、文件或树/图状数据组织、排序与模拟规则叠加，算法不偏竞赛但实现细节多。",
        _ => "中等：接近往年编程题，题面较长，需要整理对象关系和多步处理规则，主要考察结构体、数组/链式结构、排序和模拟。",
    }
}

fn chat_completions_url(api_url: &str) -> String {
    let trimmed = api_url.trim().trim_end_matches('/').to_string();
    if trimmed.contains("api.deepseek.com") && trimmed.ends_with("/v1/chat/completions") {
        return trimmed.replace("/v1/chat/completions", "/chat/completions");
    }
    if trimmed.ends_with("/chat/completions") {
        return trimmed;
    }
    if trimmed.contains("api.deepseek.com") && trimmed.ends_with("/v1") {
        return format!("{}/chat/completions", trimmed.trim_end_matches("/v1"));
    }
    format!("{trimmed}/chat/completions")
}

fn build_generation_prompt(request: &GenerateRequest) -> String {
    let topic_text = request
        .topics
        .iter()
        .map(|topic| {
            format!(
                "- [{}] {}{}\n  来源：{}\n  摘要：{}",
                topic.source,
                topic.title,
                topic
                    .year
                    .as_ref()
                    .map(|year| format!(" ({year})"))
                    .unwrap_or_default(),
                topic.path,
                topic.excerpt
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let file_io = if request.include_file_io {
        "必须包含 C 语言文件输入输出考点，返回 ioMode.kind=file，并给出 inputFile/outputFile，例如 data.in/data.out。"
    } else {
        "使用标准输入输出，返回 ioMode.kind=stdio。"
    };

    format!(
        r#"你是一个数据结构课程机试命题 agent。请生成一道原创编程题，并生成可用于本地出数据和对拍的标准正确代码。

固定命题规范：
1. 题目必须是中文题面，适合 C 语言数据结构机试。
2. 必须给出确定性参考代码，语言只能为 c11，不能生成 C++ 代码。
3. 不要直接枚举正式测试输入或输出；必须给出 dataGenerator，本地应用会编译运行该脚本生成正式测试输入，并运行 referenceSolution 得到输出答案。
4. 展示给用户的 samples 至少 2 组，必须全部是小规模、易阅读、输出不长的样例，避免题面网页被大输入或大输出撑长。样例 output 可以留空，后端会运行参考代码生成。
5. 不要输出 Markdown 代码块，不要输出解释，只输出一个 JSON 对象。
6. constraints 必须是字符串数组。
7. 如果文件输入输出，题面要明确输入文件名和输出文件名，参考代码也要实际读写该文件。
8. 题目风格必须贴近历年机试题：题面较长、规则描述细、算法思维不要太重，但实现代码量较高，重点考验理解题意、结构体建模、输入输出、排序、模拟和边界处理。
9. C 参考代码必须是完整可编译的 C11 程序，不使用 C++ 语法，不依赖非标准库。
10. dataGenerator.language 必须为 c11，code 必须是完整可编译的 C11 程序，运行后向 stdout 输出恰好 10 组测试输入。
11. dataGenerator 必须使用固定伪随机种子 srand(20260616)，保证同一题目每次本地生成完全一致；可以混合手工构造数据和伪随机数据，但不能依赖时间、文件、网络或系统环境。
12. dataGenerator 的 10 组数据结构必须是：第 1-2 组为边界/最小规模，第 3-5 组为人工构造小样例，第 6-8 组为随机中大规模，第 9-10 组为极限/卡边界大规模。
13. dataGenerator 必须覆盖容易写错的情况：重复值、相等关键字、逆序或乱序、空结果/无解输出、最大值或接近最大值；禁止 10 组都是顺序递增、形态相同的数据。
14. dataGenerator 输出多组输入时，必须在相邻两组测试输入之间单独输出一行 ---AUTO_JUDGE_CASE---，最后一组后不要再输出分隔线。每组内容必须正好是用户提交程序会读到的一次完整标准输入。
15. referenceSolution 是标准对拍代码，本地应用会对 dataGenerator 产生的每组输入运行它，得到 expected output。
16. testInputs 必须返回空数组 []；禁止把 10 组正式测试输入、正式输出答案或大样例内容写进 JSON。
17. 整个 JSON 应控制在 32000 token 以内；题面可以长，但不要复述课件，不要展开生成出的测试数据。
18. referenceSolution 和 dataGenerator 代码应完整但精炼，避免大段注释和重复代码。

固定 JSON schema：
{{
  "title": "题目标题",
  "difficulty": "easy|medium|hard",
  "statement": "问题描述",
  "inputFormat": "输入形式",
  "outputFormat": "输出形式",
  "constraints": ["约束1", "约束2"],
  "tags": ["链表", "排序"],
  "ioMode": {{
    "kind": "stdio|file",
    "inputFile": "data.in 或 null",
    "outputFile": "data.out 或 null"
  }},
  "samples": [
    {{ "input": "样例输入", "output": "" }}
  ],
  "referenceSolution": {{
    "language": "c11",
    "code": "完整可编译代码"
  }},
  "dataGenerator": {{
    "language": "c11",
    "code": "完整可编译的数据生成程序，输出多组测试输入并用分隔线分开"
  }},
  "testInputs": []
}}

本次选择考点：
{topic_text}

本次题目难度：
{difficulty}

本次输入输出模式：
{file_io}

本次补充要求：
{extra}
"#,
        extra = request.extra_requirements,
        difficulty = difficulty_prompt_label(&request.difficulty)
    )
}

fn cache_key_for_request(request: &GenerateRequest, prompt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DATASET_POLICY_VERSION.as_bytes());
    hasher.update(b"\n");
    hasher.update(chat_completions_url(&request.api_url).as_bytes());
    hasher.update(b"\n");
    hasher.update(request.model.trim().as_bytes());
    hasher.update(b"\n");
    hasher.update(prompt.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn read_cached_draft(
    app: &tauri::AppHandle,
    cache_key: &str,
) -> Result<Option<GeneratedProblemDraft>, String> {
    let path = cache_directory(app)?.join(format!("{cache_key}.json"));
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .map(Some)
            .map_err(|error| format!("Agent 缓存格式错误：{error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("无法读取 Agent 缓存：{error}")),
    }
}

fn save_cached_draft(
    app: &tauri::AppHandle,
    cache_key: &str,
    draft: &GeneratedProblemDraft,
) -> Result<(), String> {
    write_json(
        &cache_directory(app)?.join(format!("{cache_key}.json")),
        &json!(draft),
    )
}

fn extract_json_object(content: &str) -> Result<String, String> {
    let start = content
        .find('{')
        .ok_or_else(|| "模型响应中没有 JSON 对象".to_string())?;
    let end = content
        .rfind('}')
        .ok_or_else(|| "模型响应中 JSON 对象不完整".to_string())?;
    if end <= start {
        return Err("模型响应中 JSON 对象不完整".to_string());
    }
    Ok(content[start..=end].to_string())
}

fn is_probable_windows_path_escape(chars: &[char], slash_index: usize) -> bool {
    let escaped = chars.get(slash_index + 1).copied().unwrap_or_default();
    let after_escape = slash_index + 2;
    if after_escape >= chars.len() || chars[after_escape].is_whitespace() {
        return false;
    }
    if after_escape + 1 < chars.len()
        && chars[after_escape].is_ascii_alphabetic()
        && chars[after_escape + 1] == ':'
    {
        return false;
    }

    if slash_index > 0 && chars[slash_index - 1] == '\\' {
        return true;
    }

    let mut token_start = 0;
    for index in (0..slash_index).rev() {
        if chars[index].is_whitespace()
            || (index > 0 && chars[index - 1] == '\\' && matches!(chars[index], 'n' | 'r'))
        {
            token_start = index + 1;
            break;
        }
    }
    let token_prefix = &chars[token_start..slash_index];
    if token_prefix.len() >= 2 && token_prefix[0].is_ascii_alphabetic() && token_prefix[1] == ':' {
        return true;
    }

    escaped.is_ascii_alphanumeric()
        && (token_prefix.len() >= 2 && token_prefix[0] == '\\' && token_prefix[1] == '\\')
}

fn normalize_model_text_newlines(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if !normalized.contains("\\n") && !normalized.contains("\\r") {
        return normalized;
    }

    let chars = normalized.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(normalized.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\' && index + 1 < chars.len() {
            match chars[index + 1] {
                'n' if !is_probable_windows_path_escape(&chars, index) => {
                    output.push('\n');
                    index += 2;
                    continue;
                }
                'r' if !is_probable_windows_path_escape(&chars, index) => {
                    if index + 3 < chars.len()
                        && chars[index + 2] == '\\'
                        && chars[index + 3] == 'n'
                    {
                        output.push('\n');
                        index += 4;
                    } else {
                        output.push('\n');
                        index += 2;
                    }
                    continue;
                }
                _ => {}
            }
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}

fn normalize_generated_draft_text(draft: &mut GeneratedProblemDraft) {
    draft.title = normalize_model_text_newlines(&draft.title);
    draft.statement = normalize_model_text_newlines(&draft.statement);
    draft.input_format = normalize_model_text_newlines(&draft.input_format);
    draft.output_format = normalize_model_text_newlines(&draft.output_format);
    draft.constraints = draft
        .constraints
        .iter()
        .map(|item| normalize_model_text_newlines(item))
        .collect();
    draft.tags = draft
        .tags
        .iter()
        .map(|item| normalize_model_text_newlines(item))
        .collect();
    draft.samples = draft
        .samples
        .iter()
        .map(|sample| SampleCase {
            input: normalize_model_text_newlines(&sample.input),
            output: sample
                .output
                .as_ref()
                .map(|output| normalize_model_text_newlines(output)),
        })
        .collect();
    draft.test_inputs = draft
        .test_inputs
        .iter()
        .map(|input| normalize_model_text_newlines(input))
        .collect();
}

fn normalize_test_case_text(case: &mut TestCase) {
    case.input = normalize_model_text_newlines(&case.input);
    case.expected_output = normalize_model_text_newlines(&case.expected_output);
    case.files = case
        .files
        .iter()
        .map(|file| TestFile {
            name: file.name.clone(),
            content: normalize_model_text_newlines(&file.content),
        })
        .collect();
}

fn normalize_problem_record_text(record: &mut ProblemRecord) {
    record.title = normalize_model_text_newlines(&record.title);
    record.statement = normalize_model_text_newlines(&record.statement);
    record.input_format = normalize_model_text_newlines(&record.input_format);
    record.output_format = normalize_model_text_newlines(&record.output_format);
    record.constraints = record
        .constraints
        .iter()
        .map(|item| normalize_model_text_newlines(item))
        .collect();
    record.tags = record
        .tags
        .iter()
        .map(|item| normalize_model_text_newlines(item))
        .collect();
    record.topic_titles = record
        .topic_titles
        .iter()
        .map(|item| normalize_model_text_newlines(item))
        .collect();
    for case in &mut record.samples {
        normalize_test_case_text(case);
    }
    for case in &mut record.tests {
        normalize_test_case_text(case);
    }
}

async fn request_generation_draft(
    request: &GenerateRequest,
    prompt: &str,
) -> Result<GeneratedProblemDraft, String> {
    let client = Client::new();
    let endpoint = chat_completions_url(&request.api_url);
    let response = client
        .post(&endpoint)
        .bearer_auth(request.api_key.trim())
        .json(&json!({
            "model": request.model.trim(),
            "messages": [
                {
                    "role": "system",
                    "content": "你是严谨的数据结构编程题命题、出数据和对拍专家。所有输出必须可机器解析。"
                },
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "temperature": 0.4,
            "max_tokens": GENERATION_MAX_TOKENS,
            "response_format": { "type": "json_object" }
        }))
        .send()
        .await
        .map_err(|error| format!("无法请求生成 API：{error}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("无法读取生成 API 响应：{error}"))?;
    if !status.is_success() {
        return Err(format!(
            "生成 API 返回失败状态 {status}，请求地址 {endpoint}：{body}"
        ));
    }
    let value: Value =
        serde_json::from_str(&body).map_err(|error| format!("生成 API 响应不是 JSON：{error}"))?;
    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| "生成 API 响应缺少 choices[0].message.content".to_string())?;
    let finish_reason = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let json_text = extract_json_object(content)?;
    let mut draft: GeneratedProblemDraft = serde_json::from_str(&json_text).map_err(|error| {
        if finish_reason == "length" {
            format!(
                "模型输出被 max_tokens 截断，JSON 不完整：{error}。请重新生成，或降低难度/减少补充要求。"
            )
        } else {
            format!("模型 JSON 格式错误：{error}\n{json_text}")
        }
    })?;
    normalize_generated_draft_text(&mut draft);
    Ok(draft)
}

async fn call_generation_api(
    app: &tauri::AppHandle,
    request: &GenerateRequest,
) -> Result<GeneratedProblemDraft, String> {
    validate_generation_request(request)?;
    let prompt = build_generation_prompt(request);
    let cache_key = cache_key_for_request(request, &prompt);
    let mut draft = if request.use_cache {
        if let Some(draft) = read_cached_draft(app, &cache_key)? {
            draft
        } else {
            request_generation_draft(request, &prompt)
                .await
                .map_err(|error| format!("{error}"))?
        }
    } else {
        request_generation_draft(request, &prompt)
            .await
            .map_err(|error| format!("{error}"))?
    };
    normalize_generated_draft_text(&mut draft);
    if draft.data_generator.is_none() {
        return Err("模型没有返回 dataGenerator。为避免输出 token 失控，正式测试数据必须由本地生成脚本产生。".to_string());
    }
    if draft.samples.is_empty() {
        return Err("模型没有返回样例".to_string());
    }
    if draft.samples.len() < 2 {
        return Err("模型返回的展示样例少于 2 组".to_string());
    }
    if !matches!(draft.reference_solution.language.as_str(), "c11" | "c") {
        return Err("模型返回了非 C 语言参考代码，请重新生成".to_string());
    }
    if let Some(generator) = &draft.data_generator {
        if !matches!(generator.language.as_str(), "c11" | "c") {
            return Err("模型返回了非 C 语言数据生成脚本，请重新生成".to_string());
        }
    }
    draft.reference_solution.language = "c11".to_string();
    draft.difficulty = request.difficulty.clone();
    draft.test_inputs.clear();
    if draft.io_mode.kind == "file" {
        if draft
            .io_mode
            .input_file
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            draft.io_mode.input_file = Some("data.in".to_string());
        }
        if draft
            .io_mode
            .output_file
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            draft.io_mode.output_file = Some("data.out".to_string());
        }
    }
    if request.use_cache {
        save_cached_draft(app, &cache_key, &draft)?;
    }
    Ok(draft)
}

fn extension_for_language(language: &str) -> Result<&'static str, String> {
    match language {
        "c11" | "c" => Ok("c"),
        "cpp17" | "cpp" | "c++17" => Ok("cpp"),
        _ => Err(format!("不支持的语言：{language}")),
    }
}

fn compiler_for_language(language: &str) -> Result<(&'static str, Vec<&'static str>), String> {
    match language {
        "c11" | "c" => Ok(("gcc", vec!["-std=c11", "-Wall", "-Wextra", "-O2"])),
        "cpp17" | "cpp" | "c++17" => Ok(("g++", vec!["-std=c++17", "-Wall", "-Wextra", "-O2"])),
        _ => Err(format!("不支持的语言：{language}")),
    }
}

fn compile_source(
    language: &str,
    source: &str,
    prefix: &str,
) -> Result<Result<CompiledProgram, (String, String)>, String> {
    let extension = extension_for_language(language)?;
    let (compiler, args) = compiler_for_language(language)?;
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let work_dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}-{counter}",
        std::process::id(),
        now_millis()?
    ));
    fs::create_dir_all(&work_dir).map_err(|error| format!("无法创建临时目录：{error}"))?;
    let source_path = work_dir.join(format!("main.{extension}"));
    let binary_path = work_dir.join(if cfg!(windows) { "main.exe" } else { "main" });
    fs::write(&source_path, source).map_err(|error| format!("无法写入源码：{error}"))?;

    let output = Command::new(compiler)
        .args(args)
        .arg(&source_path)
        .arg("-o")
        .arg(&binary_path)
        .output()
        .map_err(|error| format!("无法调用 {compiler}，请确认本机已安装编译器：{error}"))?;
    if !output.status.success() {
        let _ = fs::remove_dir_all(&work_dir);
        return Ok(Err((
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        )));
    }
    Ok(Ok(CompiledProgram {
        work_dir,
        binary_path,
    }))
}

fn run_program(
    program: &CompiledProgram,
    input: &str,
    io_mode: &IoMode,
    files: &[TestFile],
) -> Result<RunOutput, String> {
    let stdout_path = program.work_dir.join("stdout.txt");
    let stderr_path = program.work_dir.join("stderr.txt");
    let stdout_file =
        fs::File::create(&stdout_path).map_err(|error| format!("无法创建输出文件：{error}"))?;
    let stderr_file =
        fs::File::create(&stderr_path).map_err(|error| format!("无法创建错误输出文件：{error}"))?;

    for file in files {
        let file_path = program.work_dir.join(&file.name);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("无法创建测试文件目录：{error}"))?;
        }
        fs::write(file_path, &file.content)
            .map_err(|error| format!("无法写入测试文件 {}：{error}", file.name))?;
    }

    if io_mode.kind == "file" {
        let input_file = io_mode.input_file.as_deref().unwrap_or("data.in");
        let output_file = io_mode.output_file.as_deref().unwrap_or("data.out");
        fs::write(program.work_dir.join(input_file), input)
            .map_err(|error| format!("无法写入输入文件：{error}"))?;
        let _ = fs::remove_file(program.work_dir.join(output_file));
    }

    let mut child = Command::new(&program.binary_path)
        .current_dir(&program.work_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .map_err(|error| format!("无法运行编译产物：{error}"))?;

    if let Some(mut child_stdin) = child.stdin.take() {
        child_stdin
            .write_all(input.as_bytes())
            .map_err(|error| format!("无法写入标准输入：{error}"))?;
    }

    let started = Instant::now();
    let timeout = Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("无法等待程序结束：{error}"))?
        {
            break Some(status);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(25));
    };

    let stdout = if io_mode.kind == "file" {
        let output_file = io_mode.output_file.as_deref().unwrap_or("data.out");
        fs::read_to_string(program.work_dir.join(output_file)).unwrap_or_default()
    } else {
        fs::read_to_string(&stdout_path).unwrap_or_default()
    };
    let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
    let elapsed_ms = started.elapsed().as_millis();
    let run_status = match status {
        None => "TLE",
        Some(item) if item.success() => "OK",
        Some(_) => "RE",
    }
    .to_string();

    Ok(RunOutput {
        status: run_status,
        stdout,
        stderr,
        elapsed_ms,
    })
}

fn normalize_output(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_generated_test_inputs(output: &str) -> Vec<String> {
    output
        .replace("\r\n", "\n")
        .split(&format!("\n{TEST_CASE_SEPARATOR}\n"))
        .map(|case| case.trim_matches('\n').trim_end())
        .filter(|case| !case.trim().is_empty())
        .map(|case| format!("{case}\n"))
        .collect()
}

fn test_input_scale_score(input: &str) -> usize {
    input.len() + input.lines().count() * 20 + input.split_whitespace().count() * 3
}

fn validate_generated_test_inputs(inputs: &[String]) -> Result<(), String> {
    if inputs.len() != 10 {
        return Err(format!(
            "数据生成脚本必须生成恰好 10 组正式测试输入：实际 {} 组",
            inputs.len()
        ));
    }
    let unique = inputs
        .iter()
        .map(|input| normalize_output(input))
        .collect::<HashSet<_>>();
    if unique.len() <= 1 {
        return Err("数据生成脚本生成的 10 组输入完全相同，随机性和覆盖不足".to_string());
    }

    let midpoint = inputs.len() / 2;
    let small_scores = inputs[..midpoint]
        .iter()
        .map(|input| test_input_scale_score(input))
        .collect::<Vec<_>>();
    let large_scores = inputs[midpoint..]
        .iter()
        .map(|input| test_input_scale_score(input))
        .collect::<Vec<_>>();
    let small_avg = small_scores.iter().sum::<usize>() as f64 / small_scores.len() as f64;
    let large_avg = large_scores.iter().sum::<usize>() as f64 / large_scores.len() as f64;
    let small_max = small_scores.iter().copied().max().unwrap_or(0) as f64;
    let large_max = large_scores.iter().copied().max().unwrap_or(0) as f64;

    if large_avg < small_avg * 1.5 && large_max < small_max * 2.0 {
        return Err("数据生成脚本没有体现前 5 组小样例、后 5 组大样例的规模差异".to_string());
    }
    Ok(())
}

fn run_data_generator(generator: &DataGenerator) -> Result<Vec<String>, String> {
    if !matches!(generator.language.as_str(), "c11" | "c") {
        return Err("模型返回的数据生成脚本不是 C11".to_string());
    }
    let compiled = compile_source(&generator.language, &generator.code, "auto-judge-generator")?;
    let compiled = match compiled {
        Ok(program) => program,
        Err((_, stderr)) => return Err(format!("数据生成脚本编译失败：{stderr}")),
    };
    let output = run_program(
        &compiled,
        "",
        &IoMode {
            kind: "stdio".to_string(),
            input_file: None,
            output_file: None,
        },
        &[],
    );
    let _ = fs::remove_dir_all(&compiled.work_dir);
    let output = output?;
    if output.status != "OK" {
        return Err(format!(
            "数据生成脚本运行失败：{}\n{}",
            output.status, output.stderr
        ));
    }
    Ok(parse_generated_test_inputs(&output.stdout))
}

fn test_inputs_from_draft(draft: &GeneratedProblemDraft) -> Result<Vec<String>, String> {
    if let Some(generator) = &draft.data_generator {
        let inputs = run_data_generator(generator)?;
        validate_generated_test_inputs(&inputs)?;
        return Ok(inputs);
    }
    if draft.test_inputs.len() < 10 {
        return Err("模型没有返回 dataGenerator，且正式测试输入少于 10 组".to_string());
    }
    Ok(draft.test_inputs.clone())
}

fn scaled_test_name(index: usize, total: usize) -> String {
    let midpoint = total / 2;
    if index < midpoint {
        format!("small-{:02}", index + 1)
    } else {
        format!("large-{:02}", index - midpoint + 1)
    }
}

fn apply_scaled_test_names(record: &mut ProblemRecord) {
    let total = record.tests.len();
    for (index, case) in record.tests.iter_mut().enumerate() {
        case.name = scaled_test_name(index, total);
    }
}

fn first_input_number(input: &str) -> Option<usize> {
    input.split_whitespace().next()?.parse::<usize>().ok()
}

fn builtin_cache_is_current(record: &ProblemRecord) -> bool {
    if record.tests.len() != 10 {
        return false;
    }
    if record
        .samples
        .iter()
        .any(|case| case.input.lines().count() > 12 || case.expected_output.lines().count() > 12)
    {
        return false;
    }
    match record.id.as_str() {
        "past-2018-student-online-time" => {
            first_input_number(&record.tests[9].input).is_some_and(|value| value >= 100)
        }
        "past-2021-hamming-distance" => {
            first_input_number(&record.tests[9].input).is_some_and(|value| value >= 16)
        }
        "past-2018-postfix-expression" => record.tests[9].input.len() >= 200,
        "past-2019-memory-block-merge" => {
            first_input_number(&record.tests[9].input).is_some_and(|value| value >= 100)
        }
        "past-2020-exam-login-anomaly" => {
            first_input_number(&record.tests[9].input).is_some_and(|value| value >= 200)
        }
        "past-2022-co-location" => {
            first_input_number(&record.tests[9].input).is_some_and(|value| value >= 900)
        }
        "past-2021-binary-search-tree" => {
            first_input_number(&record.tests[9].input).is_some_and(|value| value >= 200)
        }
        _ => true,
    }
}

fn problem_record_from_draft(
    id: String,
    created_at: String,
    topic_titles: Vec<String>,
    draft: GeneratedProblemDraft,
) -> Result<ProblemRecord, String> {
    let test_inputs = test_inputs_from_draft(&draft)?;
    let reference = compile_source(
        &draft.reference_solution.language,
        &draft.reference_solution.code,
        "auto-judge-reference",
    )?;
    let reference = match reference {
        Ok(program) => program,
        Err((_, stderr)) => return Err(format!("参考代码编译失败：{stderr}")),
    };

    let mut samples = Vec::new();
    for (index, sample) in draft.samples.iter().enumerate() {
        let output = run_program(&reference, &sample.input, &draft.io_mode, &[])?;
        if output.status != "OK" {
            let _ = fs::remove_dir_all(&reference.work_dir);
            return Err(format!(
                "参考代码运行样例 {} 失败：{}\n{}",
                index + 1,
                output.status,
                output.stderr
            ));
        }
        samples.push(TestCase {
            name: format!("sample-{:02}", index + 1),
            input: sample.input.clone(),
            expected_output: output.stdout,
            files: Vec::new(),
        });
    }

    let mut tests = Vec::new();
    for (index, input) in test_inputs.iter().enumerate() {
        let output = run_program(&reference, input, &draft.io_mode, &[])?;
        if output.status != "OK" {
            let _ = fs::remove_dir_all(&reference.work_dir);
            return Err(format!(
                "参考代码运行测试点 {} 失败：{}\n{}",
                index + 1,
                output.status,
                output.stderr
            ));
        }
        tests.push(TestCase {
            name: scaled_test_name(index, test_inputs.len()),
            input: input.clone(),
            expected_output: output.stdout,
            files: Vec::new(),
        });
    }
    let _ = fs::remove_dir_all(&reference.work_dir);

    Ok(ProblemRecord {
        id,
        created_at,
        title: draft.title,
        difficulty: draft.difficulty,
        statement: draft.statement,
        input_format: draft.input_format,
        output_format: draft.output_format,
        constraints: draft.constraints,
        tags: draft.tags,
        topic_titles,
        io_mode: draft.io_mode,
        samples,
        tests,
        reference_solution: draft.reference_solution,
    })
    .map(|mut record| {
        apply_scaled_test_names(&mut record);
        record
    })
}

fn build_problem_record(
    request: &GenerateRequest,
    draft: GeneratedProblemDraft,
) -> Result<ProblemRecord, String> {
    problem_record_from_draft(
        format!("problem-{}", now_millis()?),
        now_string()?,
        request
            .topics
            .iter()
            .map(|topic| topic.title.clone())
            .collect(),
        draft,
    )
}

fn student_online_input(n: usize) -> String {
    let mut rows = vec![format!("{n}\n")];
    for index in 0..n {
        let student = index % 50;
        rows.push(format!(
            "stu{:02} {:08} {}\n",
            student,
            student + 1,
            1 + ((index * 137) % 86400)
        ));
    }
    rows.concat()
}

fn hamming_input(n: usize, len: usize) -> String {
    let base = "abcdefghijklmnop".chars().take(len).collect::<String>();
    let mut rows = vec![format!("{n}\n{base}\n")];
    for index in 1..n {
        let mut chars = base.chars().collect::<Vec<_>>();
        for offset in 0..=index % len {
            let position = (index * 3 + offset) % len;
            chars[position] = (b'A' + ((index + offset) % 26) as u8) as char;
        }
        rows.push(chars.into_iter().collect::<String>());
        rows.push("\n".to_string());
    }
    rows.concat()
}

fn memory_input(n: usize) -> String {
    let mut rows = vec![format!("{n}\n")];
    let mut start = 0;
    for index in 0..n {
        let width = 3 + (index % 9) as i32;
        rows.push(format!("{start} {}\n", start + width));
        start += width + if index % 4 == 0 { 2 } else { 1 };
    }
    rows.concat()
}

fn exam_login_input(n: usize) -> String {
    let mut rows = vec![format!("{n}\n")];
    for index in 0..n {
        let id = 190000 + (index % 80);
        let machine = if index % 7 == 0 { 900 + index } else { id % 30 };
        rows.push(format!(
            "{} name{:02} {} {:06}\n",
            id,
            id % 100,
            machine,
            90000 + (index % 3600)
        ));
    }
    rows.concat()
}

fn co_location_input(n: usize) -> String {
    let target = "13557912211";
    let mut rows = vec![format!("{n}\n")];
    for index in 0..n {
        let phone = if index % 17 == 0 {
            target.to_string()
        } else {
            format!("13{:09}", 100000000 + index)
        };
        let station = (b'A' + (index % 6) as u8) as char;
        let enter = 60000 + (index % 1200);
        let leave = enter + 90 + (index % 200);
        rows.push(format!("{phone} {station} {enter:06} {leave:06}\n"));
    }
    rows.push(format!("{target}\n"));
    rows.concat()
}

fn bst_input(n: usize) -> String {
    let mut rows = vec![format!("{n}\n")];
    for index in 0..n {
        if index > 0 {
            rows.push(" ".to_string());
        }
        let value = ((index * 37) % 997) as i32 - 450;
        rows.push(value.to_string());
    }
    rows.push("\n".to_string());
    rows.concat()
}

fn postfix_input(pair_count: usize, mode: usize) -> String {
    let mut tokens = Vec::new();
    for index in 1..=pair_count {
        tokens.push((index + 10).to_string());
        tokens.push((index % 9 + 1).to_string());
        tokens.push("+".to_string());
    }
    for _ in 1..pair_count {
        tokens.push("+".to_string());
    }
    format!("{}\n{mode}\n", tokens.join(" "))
}

fn enrich_builtin_draft(id: &str, draft: &mut GeneratedProblemDraft) {
    match id {
        "past-2018-student-online-time" => {
            draft.test_inputs.splice(
                5..,
                [50, 60, 75, 90, 100].into_iter().map(student_online_input),
            );
        }
        "past-2021-hamming-distance" => {
            draft.test_inputs.splice(
                5..,
                [(12, 16), (13, 16), (14, 16), (15, 16), (16, 16)]
                    .into_iter()
                    .map(|(n, len)| hamming_input(n, len)),
            );
        }
        "past-2018-postfix-expression" => {
            draft.test_inputs.splice(
                5..,
                [(20, 1), (24, 2), (28, 1), (32, 2), (36, 2)]
                    .into_iter()
                    .map(|(pairs, mode)| postfix_input(pairs, mode)),
            );
        }
        "past-2019-memory-block-merge" => {
            draft
                .test_inputs
                .splice(5.., [60, 70, 80, 90, 100].into_iter().map(memory_input));
        }
        "past-2020-exam-login-anomaly" => {
            draft.test_inputs.splice(
                5..,
                [100, 125, 150, 175, 200].into_iter().map(exam_login_input),
            );
        }
        "past-2022-co-location" => {
            draft.test_inputs.splice(
                5..,
                [200, 350, 500, 750, 999].into_iter().map(co_location_input),
            );
        }
        "past-2021-binary-search-tree" => {
            draft
                .test_inputs
                .splice(5.., [80, 120, 160, 200, 240].into_iter().map(bst_input));
        }
        _ => {}
    }
}

fn builtin_past_problem_drafts() -> Vec<(String, String, GeneratedProblemDraft)> {
    let mut drafts = vec![
        (
            "past-2018-student-online-time".to_string(),
            "2018 学生在线上机时间统计".to_string(),
            GeneratedProblemDraft {
                title: "学生在线上机时间统计".to_string(),
                difficulty: "medium".to_string(),
                statement: "教学平台日志记录学生使用系统的姓名、学号和使用时间。同一学生可能多次登录，请按学号合并其总使用时间，并按总时间从小到大输出；总时间相同时按学号从小到大输出。".to_string(),
                input_format: "第一行输入记录条数 n（1<=n<=100）。接下来 n 行，每行包含姓名、8 位学号、使用时间秒数。不会出现同一学号对应不同姓名的情况。".to_string(),
                output_format: "分行输出每位学生的姓名、学号和合并后的使用时间，按时间升序、学号升序排列。".to_string(),
                constraints: vec![
                    "1 <= n <= 100".to_string(),
                    "姓名由 3-20 个英文字母组成".to_string(),
                    "使用时间为 1 到 86400 秒".to_string(),
                ],
                tags: vec!["历年题".to_string(), "结构体".to_string(), "排序".to_string(), "聚合统计".to_string()],
                io_mode: IoMode {
                    kind: "stdio".to_string(),
                    input_file: None,
                    output_file: None,
                },
                samples: vec![
                    SampleCase {
                        input: "10\nwanghai 19373001 3600\nliupeng 19374521 1796\nzhanghuimei 19182538 2421\nlipengyou 19230908 7329\nqinhong 19060211 650\nzhaopin 17182785 1076\nsunliang 15375026 2028\nzhanghuimei 19182538 2537\njikehong 16373890 4263\nwanghai 19373001 58\n".to_string(),
                        output: None,
                    },
                ],
                reference_solution: ReferenceSolution {
                    language: "cpp17".to_string(),
                    code: r#"#include <algorithm>
#include <iostream>
#include <map>
#include <string>
#include <tuple>
#include <vector>
using namespace std;

int main() {
    ios::sync_with_stdio(false);
    cin.tie(nullptr);

    int n;
    if (!(cin >> n)) return 0;
    map<string, pair<string, long long>> by_id;
    for (int i = 0; i < n; ++i) {
        string name, id;
        long long seconds;
        cin >> name >> id >> seconds;
        auto &entry = by_id[id];
        entry.first = name;
        entry.second += seconds;
    }
    vector<tuple<long long, string, string>> rows;
    for (const auto &item : by_id) {
        rows.emplace_back(item.second.second, item.first, item.second.first);
    }
    sort(rows.begin(), rows.end());
    for (const auto &[seconds, id, name] : rows) {
        cout << name << ' ' << id << ' ' << seconds << '\n';
    }
    return 0;
}
"#.to_string(),
                },
                data_generator: None,
                test_inputs: vec![
                    "1\nalice 00000001 1\n",
                    "3\nbob 00000002 20\namy 00000001 10\ncat 00000003 30\n",
                    "4\namy 00000001 10\namy 00000001 15\nbob 00000002 20\nbob 00000002 1\n",
                    "5\nann 10000000 50\nben 09999999 50\ncarl 10000001 49\ndora 00000001 51\neric 00000002 50\n",
                    "6\nstuA 12345678 86400\nstuB 12345679 1\nstuA 12345678 1\nstuC 00000000 86400\nstuB 12345679 2\nstuD 99999999 3\n",
                    "8\nnora 11111111 7\nnora 11111111 8\nmike 22222222 15\nlily 33333333 14\nlily 33333333 1\nzoe 00000001 15\nivan 44444444 16\nzoe 00000001 1\n",
                    "10\naaa 00000010 100\nbbb 00000009 100\nccc 00000008 100\nddd 00000007 100\neee 00000006 100\nfff 00000005 100\nggg 00000004 100\nhhh 00000003 100\niii 00000002 100\njjj 00000001 100\n",
                    "7\nalpha 88888888 300\nbeta 77777777 200\ngamma 66666666 100\nalpha 88888888 50\nbeta 77777777 50\ngamma 66666666 50\ndelta 55555555 150\n",
                    "9\npeter 12340000 9\npaul 12340001 8\nmary 12340002 7\npeter 12340000 1\npaul 12340001 2\nmary 12340002 3\njohn 12340003 10\njohn 12340003 1\nrose 12340004 11\n",
                    "12\ns01 00000001 5\ns02 00000002 4\ns03 00000003 3\ns04 00000004 2\ns05 00000005 1\ns01 00000001 5\ns02 00000002 6\ns03 00000003 7\ns04 00000004 8\ns05 00000005 9\ns06 00000006 10\ns07 00000007 10\n",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            },
        ),
        (
            "past-2021-hamming-distance".to_string(),
            "2021 汉明距离".to_string(),
            GeneratedProblemDraft {
                title: "汉明距离".to_string(),
                difficulty: "easy".to_string(),
                statement: "输入一组等长字符串，以第一个字符串为基准，统计它与其他字符串的汉明距离，并按汉明距离升序输出字符串对；距离相同则按第二个字符串的 ASCII 字典序升序输出。".to_string(),
                input_format: "第一行输入字符串个数 n（2<=n<=16），随后 n 行输入等长字符串，字符串长度不超过 16。".to_string(),
                output_format: "每行输出基准字符串、另一个字符串及其汉明距离，三者用一个空格分隔。".to_string(),
                constraints: vec!["2 <= n <= 16".to_string(), "字符串等长且长度不超过 16".to_string()],
                tags: vec!["历年题".to_string(), "字符串".to_string(), "排序".to_string()],
                io_mode: IoMode {
                    kind: "stdio".to_string(),
                    input_file: None,
                    output_file: None,
                },
                samples: vec![SampleCase {
                    input: "5\nroses\ncotes\nRoses\ncoset\nrotes\n".to_string(),
                    output: None,
                }],
                reference_solution: ReferenceSolution {
                    language: "cpp17".to_string(),
                    code: r#"#include <algorithm>
#include <iostream>
#include <string>
#include <utility>
#include <vector>
using namespace std;

int main() {
    ios::sync_with_stdio(false);
    cin.tie(nullptr);

    int n;
    if (!(cin >> n)) return 0;
    vector<string> words(n);
    for (auto &word : words) cin >> word;
    vector<pair<int, string>> rows;
    for (int i = 1; i < n; ++i) {
        int distance = 0;
        for (size_t j = 0; j < words[0].size(); ++j) {
            if (words[0][j] != words[i][j]) ++distance;
        }
        rows.push_back({distance, words[i]});
    }
    sort(rows.begin(), rows.end(), [](const auto &left, const auto &right) {
        if (left.first != right.first) return left.first < right.first;
        return left.second < right.second;
    });
    for (const auto &[distance, word] : rows) {
        cout << words[0] << ' ' << word << ' ' << distance << '\n';
    }
    return 0;
}
"#.to_string(),
                },
                data_generator: None,
                test_inputs: vec![
                    "2\na\na\n",
                    "2\na\nb\n",
                    "4\nabc\nabc\nabd\nbbc\n",
                    "5\nAAAA\nAAAB\nAABA\nABAA\nBAAA\n",
                    "5\nzzzz\naaaa\nzzza\nzzaz\nzazz\n",
                    "6\n1010\n1011\n0010\n1111\n0000\n1010\n",
                    "4\nCase\ncase\nCask\nBase\n",
                    "8\nabcd\nabce\nabcf\nxbcd\naycd\nabxd\nabcD\nzzzz\n",
                    "3\nlongword\nlongword\nLongWord\n",
                    "6\nmnopqr\nmnopqs\nmnoprr\nmnoqqr\nxnoqqr\nmnopqr\n",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            },
        ),
        (
            "past-2018-postfix-expression".to_string(),
            "2018 后缀表达式计算".to_string(),
            GeneratedProblemDraft {
                title: "后缀表达式计算".to_string(),
                difficulty: "medium".to_string(),
                statement: "输入一个合法后缀表达式，运算符只包含 +、-、*、/，运算数为非负整数。根据要求输出计算结果，或输出对应的最少括号中缀表达式与计算结果。".to_string(),
                input_format: "第一行输入后缀表达式，运算符和运算数之间用空格分隔。第二行输入 1 或 2，1 表示只输出结果，2 表示输出中缀表达式和结果。".to_string(),
                output_format: "计算结果保留小数点后两位。若要求为 2，第一行输出不含空白的中缀表达式，第二行输出计算结果。".to_string(),
                constraints: vec!["表达式长度不超过 200".to_string(), "除数不为 0".to_string(), "运算数为非负整数".to_string()],
                tags: vec!["历年题".to_string(), "栈".to_string(), "表达式".to_string()],
                io_mode: IoMode {
                    kind: "stdio".to_string(),
                    input_file: None,
                    output_file: None,
                },
                samples: vec![
                    SampleCase {
                        input: "100 25 + 27 25 - / 248 + 201 -\n1\n".to_string(),
                        output: None,
                    },
                    SampleCase {
                        input: "100 25 + 27 25 - / 248 + 201 -\n2\n".to_string(),
                        output: None,
                    },
                ],
                reference_solution: ReferenceSolution {
                    language: "cpp17".to_string(),
                    code: r#"#include <iomanip>
#include <iostream>
#include <sstream>
#include <stack>
#include <string>
#include <vector>
using namespace std;

struct Item {
    double value;
    string expr;
    int precedence;
};

bool is_operator(const string &token) {
    return token == "+" || token == "-" || token == "*" || token == "/";
}

int precedence_of(const string &op) {
    if (op == "+" || op == "-") return 1;
    if (op == "*" || op == "/") return 2;
    return 3;
}

int main() {
    ios::sync_with_stdio(false);
    cin.tie(nullptr);

    string line;
    if (!getline(cin, line)) return 0;
    int mode;
    cin >> mode;

    stringstream input(line);
    string token;
    stack<Item> items;
    while (input >> token) {
        if (!is_operator(token)) {
            items.push({stod(token), token, 3});
            continue;
        }
        Item right = items.top();
        items.pop();
        Item left = items.top();
        items.pop();
        int current = precedence_of(token);
        string left_expr = left.expr;
        string right_expr = right.expr;
        if (current > left.precedence) left_expr = "(" + left_expr + ")";
        if (current >= right.precedence) right_expr = "(" + right_expr + ")";

        double value = 0.0;
        if (token == "+") value = left.value + right.value;
        if (token == "-") value = left.value - right.value;
        if (token == "*") value = left.value * right.value;
        if (token == "/") value = left.value / right.value;
        items.push({value, left_expr + token + right_expr, current});
    }
    Item result = items.top();
    if (mode == 2) cout << result.expr << '\n';
    cout << fixed << setprecision(2) << result.value << '\n';
    return 0;
}
"#.to_string(),
                },
                data_generator: None,
                test_inputs: vec![
                    "1 2 +\n1\n",
                    "1 2 +\n2\n",
                    "10 3 /\n1\n",
                    "100 25 + 27 25 - / 248 + 201 -\n2\n",
                    "100 25 + 2 58 42 + * /\n2\n",
                    "5 1 2 + 4 * + 3 -\n2\n",
                    "8 2 / 3 4 * +\n1\n",
                    "7 2 3 * - 4 +\n2\n",
                    "50 5 / 2 3 + *\n2\n",
                    "9 3 / 2 / 5 +\n2\n",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            },
        ),
        (
            "past-2019-memory-block-merge".to_string(),
            "2019 空闲内存空间合并".to_string(),
            GeneratedProblemDraft {
                title: "空闲内存空间合并".to_string(),
                difficulty: "easy".to_string(),
                statement: "一个内存空间块用起始地址和结束地址表示。若两个空间块相邻，例如 [0,100] 与 [101,200]，则可以合并为 [0,200]。输入若干互不重叠的空闲空间块，合并所有相邻空间块，并按起始地址升序输出。".to_string(),
                input_format: "第一行输入空间块个数 n（1<=n<=100）。接下来 n 行每行输入一个空间块的起始地址和结束地址，地址为 0 到 100000 的整数，输入空间块不会重叠。".to_string(),
                output_format: "输出合并后的空间块，每行两个整数表示起始地址和结束地址，按起始地址升序排列。".to_string(),
                constraints: vec!["1 <= n <= 100".to_string(), "0 <= start <= end <= 100000".to_string(), "空间块之间不存在重叠".to_string()],
                tags: vec!["历年题".to_string(), "排序".to_string(), "区间合并".to_string()],
                io_mode: IoMode {
                    kind: "stdio".to_string(),
                    input_file: None,
                    output_file: None,
                },
                samples: vec![SampleCase {
                    input: "10\n48 99\n0 39\n1024 2047\n100 479\n4000 5999\n600 799\n40 47\n2048 3047\n840 859\n8000 8999\n".to_string(),
                    output: None,
                }],
                reference_solution: ReferenceSolution {
                    language: "cpp17".to_string(),
                    code: r#"#include <algorithm>
#include <iostream>
#include <utility>
#include <vector>
using namespace std;

int main() {
    ios::sync_with_stdio(false);
    cin.tie(nullptr);

    int n;
    if (!(cin >> n)) return 0;
    vector<pair<int, int>> blocks(n);
    for (auto &block : blocks) cin >> block.first >> block.second;
    sort(blocks.begin(), blocks.end());
    vector<pair<int, int>> merged;
    for (auto [left, right] : blocks) {
        if (merged.empty() || merged.back().second + 1 < left) {
            merged.push_back({left, right});
        } else {
            merged.back().second = max(merged.back().second, right);
        }
    }
    for (auto [left, right] : merged) cout << left << ' ' << right << '\n';
    return 0;
}
"#.to_string(),
                },
                data_generator: None,
                test_inputs: vec![
                    "1\n0 0\n",
                    "2\n0 10\n11 20\n",
                    "2\n0 10\n12 20\n",
                    "4\n5 9\n0 4\n10 10\n20 25\n",
                    "5\n100 200\n0 50\n52 60\n51 51\n201 300\n",
                    "6\n10 19\n30 39\n20 29\n0 9\n50 59\n40 49\n",
                    "7\n0 0\n2 2\n1 1\n4 4\n6 6\n5 5\n3 3\n",
                    "8\n1000 1999\n0 99\n200 299\n100 199\n300 399\n500 599\n400 499\n900 999\n",
                    "10\n0 10\n100 110\n11 20\n111 120\n50 60\n61 70\n71 80\n200 210\n211 211\n212 220\n",
                    "12\n5 5\n7 9\n6 6\n20 25\n26 30\n0 4\n100 100\n101 102\n104 105\n103 103\n1000 2000\n2001 3000\n",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            },
        ),
        (
            "past-2020-exam-login-anomaly".to_string(),
            "2020 机试异常检测".to_string(),
            GeneratedProblemDraft {
                title: "机试异常检测".to_string(),
                difficulty: "medium".to_string(),
                statement: "考试开始后，如果同一账号在不同机器上登录，系统认为该账号存在异常；同一账号在同一机器上多次登录属于正常。输入按登录先后顺序排列的日志，输出发生异常的账号信息。".to_string(),
                input_format: "第一行输入日志条数 n（不超过 200）。接下来 n 行每行包含学号、姓名、机器号和 6 位登录时间。".to_string(),
                output_format: "按学号从小到大输出异常账号的学号和姓名。若没有异常账号，则不输出任何内容。".to_string(),
                constraints: vec!["n <= 200".to_string(), "姓名不含空白且长度不超过 15".to_string(), "学号与机器号均不超过 int 范围".to_string()],
                tags: vec!["历年题".to_string(), "映射".to_string(), "集合".to_string(), "排序".to_string()],
                io_mode: IoMode {
                    kind: "stdio".to_string(),
                    input_file: None,
                    output_file: None,
                },
                samples: vec![SampleCase {
                    input: "6\n191028 wangdi 15 093000\n192387 litong 39 093000\n197583 huangqinian 196 093004\n197583 huangqinian 197 093008\n192387 litong 39 093009\n191028 wangdi 15 093507\n".to_string(),
                    output: None,
                }],
                reference_solution: ReferenceSolution {
                    language: "cpp17".to_string(),
                    code: r#"#include <iostream>
#include <map>
#include <set>
#include <string>
using namespace std;

int main() {
    ios::sync_with_stdio(false);
    cin.tie(nullptr);

    int n;
    if (!(cin >> n)) return 0;
    map<long long, string> names;
    map<long long, set<long long>> machines;
    for (int i = 0; i < n; ++i) {
        long long id, machine;
        string name, time;
        cin >> id >> name >> machine >> time;
        names[id] = name;
        machines[id].insert(machine);
    }
    for (const auto &[id, machine_set] : machines) {
        if (machine_set.size() > 1) cout << id << ' ' << names[id] << '\n';
    }
    return 0;
}
"#.to_string(),
                },
                data_generator: None,
                test_inputs: vec![
                    "1\n1 aaa 10 090000\n",
                    "2\n1 aaa 10 090000\n1 aaa 10 091000\n",
                    "2\n1 aaa 10 090000\n1 aaa 11 091000\n",
                    "4\n2 bbb 3 090000\n1 aaa 1 090001\n2 bbb 4 090002\n3 ccc 5 090003\n",
                    "5\n5 eee 1 090000\n4 ddd 2 090000\n5 eee 1 090100\n4 ddd 3 090200\n6 fff 4 090300\n",
                    "6\n10 tom 1 090000\n9 bob 1 090001\n8 ann 2 090002\n10 tom 2 090003\n9 bob 1 090004\n8 ann 3 090005\n",
                    "8\n100 a 1 090000\n101 b 2 090001\n102 c 3 090002\n103 d 4 090003\n100 a 1 090004\n101 b 5 090005\n102 c 3 090006\n103 d 6 090007\n",
                    "10\n200 p 7 090000\n199 q 8 090001\n198 r 9 090002\n197 s 10 090003\n196 t 11 090004\n200 p 12 090005\n198 r 9 090006\n196 t 13 090007\n197 s 10 090008\n199 q 8 090009\n",
                    "3\n2147483647 maxid 1 090000\n2147483647 maxid 2 090001\n1 minid 1 090002\n",
                    "12\n1 a 1 090001\n2 b 2 090002\n3 c 3 090003\n4 d 4 090004\n5 e 5 090005\n6 f 6 090006\n1 a 1 090007\n2 b 20 090008\n3 c 3 090009\n4 d 40 090010\n5 e 5 090011\n6 f 60 090012\n",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            },
        ),
        (
            "past-2022-co-location".to_string(),
            "2022 查找同时空人员".to_string(),
            GeneratedProblemDraft {
                title: "查找同时空人员".to_string(),
                difficulty: "medium".to_string(),
                statement: "给定多个手机进入和离开 6 个基站的日志，以及一个目标手机号。若其他手机号与目标手机号在同一基站的时间区间有重叠，则认为二者同时空。输出所有与目标手机号同时空的手机号及基站。".to_string(),
                input_format: "第一行输入日志条数 n（小于 1000）。接下来 n 行输入手机号、基站编号、进入时间、离开时间。最后一行输入目标手机号。".to_string(),
                output_format: "按手机号从大到小输出同时空手机号及基站；手机号相同则按基站字母序输出。".to_string(),
                constraints: vec!["手机号为 11 位数字字符串".to_string(), "基站为 A-F".to_string(), "进入和离开时间为 6 位数字串".to_string()],
                tags: vec!["历年题".to_string(), "区间重叠".to_string(), "排序".to_string(), "字符串".to_string()],
                io_mode: IoMode {
                    kind: "stdio".to_string(),
                    input_file: None,
                    output_file: None,
                },
                samples: vec![SampleCase {
                    input: "6\n13557912211 B 080000 090000\n18222336979 B 083000 091000\n13810013509 C 080000 090000\n13985992766 B 091001 100000\n15857596331 D 000201 235051\n13877882206 C 003123 220806\n13557912211\n".to_string(),
                    output: None,
                }],
                reference_solution: ReferenceSolution {
                    language: "cpp17".to_string(),
                    code: r#"#include <algorithm>
#include <iostream>
#include <set>
#include <string>
#include <tuple>
#include <vector>
using namespace std;

struct Log {
    string phone;
    char station;
    string enter_time;
    string leave_time;
};

bool overlap(const Log &a, const Log &b) {
    return a.station == b.station && a.enter_time <= b.leave_time && b.enter_time <= a.leave_time;
}

int main() {
    ios::sync_with_stdio(false);
    cin.tie(nullptr);

    int n;
    if (!(cin >> n)) return 0;
    vector<Log> logs(n);
    for (auto &log : logs) cin >> log.phone >> log.station >> log.enter_time >> log.leave_time;
    string target;
    cin >> target;
    vector<Log> target_logs;
    for (const auto &log : logs) {
        if (log.phone == target) target_logs.push_back(log);
    }
    set<pair<string, char>> answers;
    for (const auto &log : logs) {
        if (log.phone == target) continue;
        for (const auto &target_log : target_logs) {
            if (overlap(log, target_log)) {
                answers.insert({log.phone, log.station});
                break;
            }
        }
    }
    vector<pair<string, char>> rows(answers.begin(), answers.end());
    sort(rows.begin(), rows.end(), [](const auto &left, const auto &right) {
        if (left.first != right.first) return left.first > right.first;
        return left.second < right.second;
    });
    for (const auto &[phone, station] : rows) cout << phone << ' ' << station << '\n';
    return 0;
}
"#.to_string(),
                },
                data_generator: None,
                test_inputs: vec![
                    "1\n11111111111 A 000000 010000\n11111111111\n",
                    "2\n11111111111 A 000000 010000\n22222222222 A 003000 020000\n11111111111\n",
                    "2\n11111111111 A 000000 010000\n22222222222 B 003000 020000\n11111111111\n",
                    "3\n11111111111 A 000000 010000\n22222222222 A 010000 020000\n33333333333 A 010001 020000\n11111111111\n",
                    "4\n11111111111 A 100000 110000\n22222222222 A 090000 095959\n33333333333 A 090000 100000\n44444444444 A 110000 120000\n11111111111\n",
                    "6\n11111111111 A 000000 010000\n11111111111 B 020000 030000\n22222222222 A 003000 004000\n22222222222 B 010000 015959\n33333333333 B 025000 040000\n44444444444 C 000000 235959\n11111111111\n",
                    "7\n55555555555 C 120000 130000\n11111111111 C 125959 140000\n99999999999 C 110000 120000\n88888888888 C 130000 140000\n77777777777 D 120000 130000\n66666666666 C 140001 150000\n55555555555 C 150000 160000\n11111111111\n",
                    "8\n12345678901 A 000000 235959\n11111111111 A 010000 020000\n22222222222 A 020001 030000\n33333333333 A 015959 020001\n44444444444 B 010000 020000\n55555555555 A 000000 010000\n66666666666 A 030000 040000\n77777777777 A 020000 020000\n11111111111\n",
                    "5\n11111111111 F 080000 090000\n22222222222 F 070000 075959\n33333333333 F 090001 100000\n44444444444 F 085959 090001\n55555555555 F 080000 080000\n11111111111\n",
                    "10\n11111111111 A 010000 020000\n11111111111 A 030000 040000\n22222222222 A 015000 015100\n22222222222 A 035000 035100\n33333333333 A 020001 025959\n44444444444 A 025959 030000\n55555555555 B 015000 035000\n66666666666 A 000000 010000\n77777777777 A 040000 050000\n88888888888 A 050001 060000\n11111111111\n",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            },
        ),
        (
            "past-2021-binary-search-tree".to_string(),
            "2021 二叉搜索树".to_string(),
            GeneratedProblemDraft {
                title: "二叉搜索树".to_string(),
                difficulty: "medium".to_string(),
                statement: "按输入顺序构造二叉查找树。插入过程中，若输入整数等于已有结点值，则该结点出现次数加一。统计建树与查找过程中的总比较次数，并输出出现次数最多的整数的比较路径；若有多个出现次数最多的整数，输出前序遍历最先访问到的那个结点路径。".to_string(),
                input_format: "第一行输入整数个数 n（n>=1），第二行输入 n 个整数。".to_string(),
                output_format: "第一行输出总比较次数。第二行输出从根结点到目标结点的路径，整数之间以一个空格分隔。".to_string(),
                constraints: vec!["n >= 1".to_string(), "输入整数在 int 范围内".to_string()],
                tags: vec!["历年题".to_string(), "二叉搜索树".to_string(), "查找".to_string(), "前序遍历".to_string()],
                io_mode: IoMode {
                    kind: "stdio".to_string(),
                    input_file: None,
                    output_file: None,
                },
                samples: vec![SampleCase {
                    input: "12\n670 1360 1871 921 128 1871 57 -200 1003 552 -200 57\n".to_string(),
                    output: None,
                }],
                reference_solution: ReferenceSolution {
                    language: "cpp17".to_string(),
                    code: r#"#include <iostream>
#include <memory>
#include <vector>
using namespace std;

struct Node {
    int value;
    int count;
    unique_ptr<Node> left;
    unique_ptr<Node> right;
    explicit Node(int v) : value(v), count(1) {}
};

void insert(unique_ptr<Node> &node, int value, long long &comparisons) {
    if (!node) {
        node = make_unique<Node>(value);
        return;
    }
    ++comparisons;
    if (value == node->value) {
        ++node->count;
    } else if (value < node->value) {
        insert(node->left, value, comparisons);
    } else {
        insert(node->right, value, comparisons);
    }
}

void first_max_preorder(const unique_ptr<Node> &node, int &best_value, int &best_count) {
    if (!node) return;
    if (node->count > best_count) {
        best_count = node->count;
        best_value = node->value;
    }
    first_max_preorder(node->left, best_value, best_count);
    first_max_preorder(node->right, best_value, best_count);
}

bool path_to(const unique_ptr<Node> &node, int target, vector<int> &path) {
    if (!node) return false;
    path.push_back(node->value);
    if (target == node->value) return true;
    if (target < node->value) {
        if (path_to(node->left, target, path)) return true;
    } else {
        if (path_to(node->right, target, path)) return true;
    }
    path.pop_back();
    return false;
}

int main() {
    ios::sync_with_stdio(false);
    cin.tie(nullptr);

    int n;
    if (!(cin >> n)) return 0;
    unique_ptr<Node> root;
    long long comparisons = 0;
    for (int i = 0; i < n; ++i) {
        int value;
        cin >> value;
        insert(root, value, comparisons);
    }
    int best_value = 0;
    int best_count = 0;
    first_max_preorder(root, best_value, best_count);
    vector<int> path;
    path_to(root, best_value, path);
    cout << comparisons << '\n';
    for (size_t i = 0; i < path.size(); ++i) {
        if (i) cout << ' ';
        cout << path[i];
    }
    cout << '\n';
    return 0;
}
"#.to_string(),
                },
                data_generator: None,
                test_inputs: vec![
                    "1\n5\n",
                    "3\n5 5 5\n",
                    "5\n5 3 7 3 7\n",
                    "7\n4 2 6 1 3 5 7\n",
                    "8\n10 5 15 5 15 15 3 3\n",
                    "10\n0 -1 1 -2 2 -3 3 -3 3 0\n",
                    "6\n1 2 3 4 5 5\n",
                    "6\n6 5 4 3 2 2\n",
                    "12\n670 1360 1871 921 128 1871 57 -200 1003 552 -200 57\n",
                    "15\n8 4 12 2 6 10 14 6 10 10 2 2 14 14 14\n",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            },
        ),
    ];
    for (id, _, draft) in &mut drafts {
        enrich_builtin_draft(id, draft);
    }
    drafts
}

fn static_case(name: &str, input: &str, expected_output: &str) -> TestCase {
    TestCase {
        name: name.to_string(),
        input: input.to_string(),
        expected_output: expected_output.to_string(),
        files: Vec::new(),
    }
}

fn static_file_case(
    name: &str,
    input: &str,
    expected_output: &str,
    file_name: &str,
    content: &str,
) -> TestCase {
    TestCase {
        name: name.to_string(),
        input: input.to_string(),
        expected_output: expected_output.to_string(),
        files: vec![TestFile {
            name: file_name.to_string(),
            content: content.to_string(),
        }],
    }
}

fn static_problem(
    id: &str,
    label: &str,
    title: &str,
    difficulty: &str,
    statement: &str,
    input_format: &str,
    output_format: &str,
    tags: &[&str],
    samples: Vec<TestCase>,
    tests: Vec<TestCase>,
) -> ProblemRecord {
    let total_tests = tests.len();
    let tests = tests
        .into_iter()
        .enumerate()
        .map(|(index, case)| TestCase {
            name: scaled_test_name(index, total_tests),
            ..case
        })
        .collect();
    let mut record = ProblemRecord {
        id: id.to_string(),
        created_at: "builtin".to_string(),
        title: title.to_string(),
        difficulty: difficulty.to_string(),
        statement: statement.to_string(),
        input_format: input_format.to_string(),
        output_format: output_format.to_string(),
        constraints: vec![
            "本题为 ds_lz 历年题内置数据，测试点随程序本地保存。".to_string(),
            "用户提交仅支持 C11。".to_string(),
        ],
        tags: std::iter::once("历年题".to_string())
            .chain(tags.iter().map(|tag| (*tag).to_string()))
            .collect(),
        topic_titles: vec![label.to_string()],
        io_mode: IoMode {
            kind: "stdio".to_string(),
            input_file: None,
            output_file: None,
        },
        samples,
        tests,
        reference_solution: ReferenceSolution {
            language: "c11".to_string(),
            code: "/* 内置历年题的样例与测试输出已预生成；提交评测只支持 C11。 */\nint main(void){return 0;}\n"
                .to_string(),
        },
    };
    apply_scaled_test_names(&mut record);
    record
}

fn extra_static_past_problems() -> Vec<ProblemRecord> {
    vec![
        static_problem(
            "past-2018-network-printer",
            "2018 网络打印机选择",
            "网络打印机选择",
            "hard",
            "网络设备按树形结构连接，配置表保存在当前目录 in.txt。标准输入给出网络设备总数和需要打印文档的计算机编号。程序需要在所有打印机中选择与该计算机距离最近的一台；若距离相同，选择配置表中先出现的打印机。输出打印机编号，以及从该计算机到打印机路径上经过的交换机编号。",
            "标准输入包含设备总数 n 和计算机编号。in.txt 每行包含设备编号、父设备编号、类型和端口号，类型 0 为交换机，1 为计算机，2 为打印机。",
            "输出所选打印机编号，后接路径上的交换机编号，编号间以一个空格分隔。",
            &["树", "文件输入", "路径"],
            vec![static_file_case(
                "sample-01",
                "3 1\n",
                "2\n",
                "in.txt",
                "0 -1 0 -1\n1 0 1 0\n2 0 2 1\n",
            )],
            vec![
                static_file_case("test-01", "3 1\n", "2\n", "in.txt", "0 -1 0 -1\n1 0 1 0\n2 0 2 1\n"),
                static_file_case("test-02", "4 2\n", "3 1\n", "in.txt", "0 -1 0 -1\n1 0 0 0\n2 1 1 0\n3 1 2 1\n"),
                static_file_case("test-03", "5 2\n", "3 1\n", "in.txt", "0 -1 0 -1\n1 0 0 0\n2 1 1 0\n3 1 2 1\n4 0 2 2\n"),
                static_file_case("test-04", "6 5\n", "4 3\n", "in.txt", "0 -1 0 -1\n1 0 0 0\n2 1 2 0\n3 1 0 1\n4 3 2 0\n5 3 1 1\n"),
                static_file_case("test-05", "6 4\n", "3 2\n", "in.txt", "0 -1 0 -1\n1 0 0 0\n2 1 0 0\n3 2 2 0\n4 2 1 1\n5 0 2 2\n"),
                static_file_case("test-06", "4 3\n", "1 2\n", "in.txt", "0 -1 0 -1\n1 0 2 0\n2 0 0 1\n3 2 1 0\n"),
                static_file_case("test-07", "7 6\n", "4 2\n", "in.txt", "0 -1 0 -1\n1 0 0 0\n2 1 0 0\n3 2 1 0\n4 2 2 1\n5 0 0 1\n6 5 1 0\n"),
                static_file_case("test-08", "8 6\n", "7 5\n", "in.txt", "0 -1 0 -1\n1 0 0 0\n2 1 2 0\n3 1 1 1\n4 0 0 1\n5 4 0 0\n6 5 1 0\n7 5 2 1\n"),
                static_file_case("test-09", "5 1\n", "2\n", "in.txt", "0 -1 0 -1\n1 0 1 0\n2 0 2 1\n3 0 0 2\n4 3 2 0\n"),
                static_file_case("test-10", "5 4\n", "2 1\n", "in.txt", "0 -1 0 -1\n1 0 0 0\n2 1 2 0\n3 1 0 1\n4 3 1 0\n"),
            ],
        ),
        static_problem(
            "past-2019-train-dispatch",
            "2019 火车货运调度模拟",
            "火车货运调度模拟",
            "hard",
            "某火车货场由 A、B、C 三段组成。现有一列货车停在 A 段上，每节车厢有编号和目的地。需要将车厢按目的地里程由远到近重新编组，目的地相同的车厢保持原始先后关系。本内置数据同时输出一个确定的 A 段 push 操作统计值，用于训练按规则读入、建表、排序和输出。",
            "先输入目的地个数及目的地、里程，再输入车厢数及车厢编号、目的地。",
            "第一行输出编组后的车厢编号，第二行输出 A 段 push 操作次数。",
            &["栈", "模拟", "排序"],
            vec![static_case(
                "sample-01",
                "3\na 10\nb 20\nc 30\n4\n1000 c\n1001 b\n1002 a\n1003 c\n",
                "1000 1003 1001 1002\n4\n",
            )],
            vec![
                static_case("test-01", "2\na 1\nb 2\n3\n0001 a\n0002 b\n0003 a\n", "0002 0001 0003\n3\n"),
                static_case("test-02", "3\na 10\nb 20\nc 30\n4\n1000 c\n1001 b\n1002 a\n1003 c\n", "1000 1003 1001 1002\n4\n"),
                static_case("test-03", "1\nonly 5\n3\n1111 only\n2222 only\n3333 only\n", "1111 2222 3333\n3\n"),
                static_case("test-04", "3\na 1\nb 2\nc 3\n5\n0001 a\n0002 a\n0003 b\n0004 c\n0005 b\n", "0004 0003 0005 0001 0002\n5\n"),
                static_case("test-05", "4\na 1\nb 2\nc 3\nd 4\n6\n0001 d\n0002 c\n0003 b\n0004 a\n0005 d\n0006 c\n", "0001 0005 0002 0006 0003 0004\n6\n"),
                static_case("test-06", "2\nnear 10\nfar 20\n1\n9999 far\n", "9999\n1\n"),
                static_case("test-07", "3\nx 100\ny 200\nz 300\n6\n1001 x\n1002 y\n1003 z\n1004 x\n1005 y\n1006 z\n", "1003 1006 1002 1005 1001 1004\n6\n"),
                static_case("test-08", "4\np 1\nq 2\nr 3\ns 4\n8\n0001 p\n0002 q\n0003 r\n0004 s\n0005 s\n0006 r\n0007 q\n0008 p\n", "0004 0005 0003 0006 0002 0007 0001 0008\n8\n"),
                static_case("test-09", "5\na 1\nb 5\nc 10\nd 20\ne 30\n5\n0001 b\n0002 e\n0003 a\n0004 d\n0005 c\n", "0002 0004 0005 0001 0003\n5\n"),
                static_case("test-10", "2\na 1\nb 2\n2\n0100 b\n0101 a\n", "0100 0101\n2\n"),
            ],
        ),
        static_problem(
            "past-2019-find-same-file",
            "2019 查找同名文件",
            "查找同名文件",
            "hard",
            "操作系统中的目录和文件按树形组织。给定一个文件名，请在 files.txt 描述的目录树中查找所有同名普通文件，输出从根目录开始的完整路径。输出顺序按修改时间由近到远；修改时间相同，按路径层次数由小到大；仍相同则按配置文件中出现的先后顺序。",
            "标准输入给出结点总数和待查找文件名。files.txt 每行给出 name、parentName、type、date。",
            "分行输出完整路径，根目录名后加冒号，目录间用反斜杠分隔。",
            &["树", "文件输入", "排序"],
            vec![static_file_case(
                "sample-01",
                "3 a.txt\n",
                "D:\\a.txt\n",
                "files.txt",
                "D - 1 20200101\na.txt D 0 20200102\nb.txt D 0 20200103\n",
            )],
            vec![
                static_file_case("test-01", "3 a.txt\n", "D:\\a.txt\n", "files.txt", "D - 1 20200101\na.txt D 0 20200102\nb.txt D 0 20200103\n"),
                static_file_case("test-02", "5 x.c\n", "C:\\src\\x.c\nC:\\x.c\n", "files.txt", "C - 1 20200101\nsrc C 1 20200101\nx.c C 0 20200102\nx.c src 0 20200103\ny.c src 0 20200104\n"),
                static_file_case("test-03", "4 none.c\n", "", "files.txt", "D - 1 1\na D 1 1\nb a 0 2\nc D 0 3\n"),
                static_file_case("test-04", "6 f\n", "D:\\f\nD:\\a\\f\nD:\\b\\f\n", "files.txt", "D - 1 1\na D 1 1\nb D 1 1\nf a 0 20200101\nf b 0 20200101\nf D 0 20200101\n"),
                static_file_case("test-05", "5 readme.md\n", "D:\\readme.md\nD:\\doc\\readme.md\n", "files.txt", "D - 1 1\ndoc D 1 1\nreadme.md doc 0 20221212\nreadme.md D 0 20230101\ntmp D 1 1\n"),
                static_file_case("test-06", "6 q.txt\n", "D:\\c\\q.txt\nD:\\q.txt\n", "files.txt", "D - 1 1\na D 1 1\nb a 1 1\nc b 1 1\nq.txt c 0 9\nq.txt D 0 8\n"),
                static_file_case("test-07", "5 same\n", "D:\\a\\same\nD:\\b\\same\n", "files.txt", "D - 1 1\na D 1 1\nsame a 0 10\nb D 1 1\nsame b 0 10\n"),
                static_file_case("test-08", "4 z\n", "D:\\z\n", "files.txt", "D - 1 1\na D 1 1\nz D 0 8\nb D 0 9\n"),
                static_file_case("test-09", "4 b\n", "D:\\b\n", "files.txt", "D - 1 1\na D 1 1\nb D 0 8\nb a 1 9\n"),
                static_file_case("test-10", "5 k\n", "R:\\a\\k\nR:\\b\\k\n", "files.txt", "R - 1 1\na R 1 1\nb R 1 1\nk a 0 7\nk b 0 6\n"),
            ],
        ),
        static_problem(
            "past-2021-mini-interpreter",
            "2021 解释系统",
            "解释系统",
            "hard",
            "有一 min 解释语言，只包含整型常量、单字母变量、赋值语句、算术表达式语句、print 语句和 exit 语句。赋值语句中无空白符，表达式包含 +、-、*、/ 和小括号。程序需要逐行解释执行，并在遇到 print 时输出变量当前值。",
            "逐行输入语句，最后一行为 exit。",
            "每条 print 语句输出变量值，保留小数点后两位。",
            &["表达式", "解释器", "模拟"],
            vec![static_case(
                "sample-01",
                "a=10\nb=20\nc=(a+b)/4\nprint a b c\nd=a*(b-c)\nprint d\nexit\n",
                "10.00 20.00 7.50\n125.00\n",
            )],
            vec![
                static_case("test-01", "a=1\nprint a\nexit\n", "1.00\n"),
                static_case("test-02", "a=1\nb=2\nprint a b\nexit\n", "1.00 2.00\n"),
                static_case("test-03", "a=10\nb=a*3+2\nprint b\nexit\n", "32.00\n"),
                static_case("test-04", "a=8\nb=2\nc=a/b\nprint c\nexit\n", "4.00\n"),
                static_case("test-05", "a=5\nb=(a+3)*(a-1)\nprint a b\nexit\n", "5.00 32.00\n"),
                static_case("test-06", "x=100\ny=x/3\nprint y\nexit\n", "33.33\n"),
                static_case("test-07", "a=1\nb=2\nc=3\nd=a+b*c\nprint d a c\nexit\n", "7.00 1.00 3.00\n"),
                static_case("test-08", "m=20\nn=6\np=(m-n)/(n-2)\nprint p\nexit\n", "3.50\n"),
                static_case("test-09", "a=10\nb=4\nc=a/b\nd=c*3\nprint c d\nexit\n", "2.50 7.50\n"),
                static_case("test-10", "a=2\nb=3\nc=(a+b)*(a+b)\nprint c\nexit\n", "25.00\n"),
            ],
        ),
        static_problem(
            "past-2022-file-copy",
            "2022 文件拷贝",
            "文件拷贝",
            "hard",
            "已有目录树保存在 in.txt 中，标准输入给出一组带完整路径和日期时间的文件。若路径上的目录不存在则创建；若同名文件不存在则拷贝；若同名文件存在且新文件日期较新则覆盖，否则保持原文件。最后按层次遍历规则输出所有普通文件。",
            "in.txt 保存已有目录树和结点属性；标准输入读入待拷贝文件数量以及每个文件的完整路径、日期时间。",
            "按层次输出普通文件名和日期时间，同层按文件名字典序，文件名相同按时间序。",
            &["树", "文件输入", "模拟"],
            vec![static_file_case("sample-01", "1\nD:\\a.txt 202201010101\n", "a.txt 202201010101\n", "in.txt", "1\n1 D: 1 -\n")],
            vec![
                static_file_case("test-01", "1\nD:\\a.txt 202201010101\n", "a.txt 202201010101\n", "in.txt", "1\n1 D: 1 -\n"),
                static_file_case("test-02", "1\nD:\\a.txt 202101010101\n", "a.txt 202201010101\n", "in.txt", "1(2)\n1 D: 1 -\n2 a.txt 0 202201010101\n"),
                static_file_case("test-03", "1\nD:\\a.txt 202301010101\n", "a.txt 202301010101\n", "in.txt", "1(2)\n1 D: 1 -\n2 a.txt 0 202201010101\n"),
                static_file_case("test-04", "2\nD:\\doc\\a.c 202201010101\nD:\\doc\\b.c 202201010102\n", "a.c 202201010101\nb.c 202201010102\n", "in.txt", "1(2)\n1 D: 1 -\n2 doc 1 -\n"),
                static_file_case("test-05", "2\nD:\\x\\y\\z.txt 202201010101\nD:\\x\\a.txt 202201010102\n", "a.txt 202201010102\nz.txt 202201010101\n", "in.txt", "1\n1 D: 1 -\n"),
                static_file_case("test-06", "3\nD:\\temp\\a 3\nD:\\temp\\a 2\nD:\\temp\\b 1\n", "a 3\nb 1\n", "in.txt", "1(2)\n1 D: 1 -\n2 temp 1 -\n"),
                static_file_case("test-07", "1\nD:\\doc\\old.txt 202001010101\n", "old.txt 202001010101\n", "in.txt", "1(2(3))\n1 D: 1 -\n2 doc 1 -\n3 old.txt 0 201901010101\n"),
                static_file_case("test-08", "2\nD:\\b.txt 2\nD:\\a.txt 3\n", "a.txt 3\nb.txt 2\n", "in.txt", "1\n1 D: 1 -\n"),
                static_file_case("test-09", "1\nD:\\d1\\d2\\d3\\f 9\n", "f 9\n", "in.txt", "1\n1 D: 1 -\n"),
                static_file_case("test-10", "1\nD:\\root.dat 5\n", "root.dat 5\n", "in.txt", "1\n1 D: 1 -\n"),
            ],
        ),
    ]
}

#[cfg(test)]
fn builtin_past_problems() -> Result<Vec<ProblemRecord>, String> {
    let mut problems = builtin_past_problem_drafts()
        .into_iter()
        .map(|(id, label, draft)| {
            problem_record_from_draft(id, "builtin".to_string(), vec![label], draft)
        })
        .collect::<Result<Vec<_>, _>>()?;
    problems.extend(extra_static_past_problems());
    Ok(problems)
}

fn builtin_past_problem(
    app: &tauri::AppHandle,
    problem_id: &str,
) -> Result<Option<ProblemRecord>, String> {
    let cache = BUILTIN_PAST_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(problem) = cache
        .lock()
        .map_err(|_| "内置历年题缓存锁定失败".to_string())?
        .get(problem_id)
        .cloned()
    {
        return Ok(Some(problem));
    }

    for problem in extra_static_past_problems() {
        if problem.id == problem_id {
            cache
                .lock()
                .map_err(|_| "内置历年题缓存锁定失败".to_string())?
                .insert(problem.id.clone(), problem.clone());
            return Ok(Some(problem));
        }
    }

    let cache_path = builtin_problem_directory(app)?.join(format!("{problem_id}.json"));
    if let Ok(content) = fs::read_to_string(&cache_path) {
        let mut problem = serde_json::from_str::<ProblemRecord>(&content)
            .map_err(|error| format!("内置历年题本地缓存格式错误：{error}"))?;
        apply_scaled_test_names(&mut problem);
        if builtin_cache_is_current(&problem) {
            cache
                .lock()
                .map_err(|_| "内置历年题缓存锁定失败".to_string())?
                .insert(problem_id.to_string(), problem.clone());
            return Ok(Some(problem));
        }
        let _ = fs::remove_file(&cache_path);
    }

    for (id, label, draft) in builtin_past_problem_drafts() {
        if id == problem_id {
            let problem =
                problem_record_from_draft(id.clone(), "builtin".to_string(), vec![label], draft)?;
            write_json(&cache_path, &json!(problem))?;
            cache
                .lock()
                .map_err(|_| "内置历年题缓存锁定失败".to_string())?
                .insert(id, problem.clone());
            return Ok(Some(problem));
        }
    }
    Ok(None)
}

fn past_problem_index() -> Result<Vec<HistoryEntry>, String> {
    let mut entries = builtin_past_problem_drafts()
        .into_iter()
        .map(|(id, label, draft)| HistoryEntry {
            id,
            title: draft.title,
            created_at: "builtin".to_string(),
            difficulty: draft.difficulty,
            topic_titles: vec![label],
            test_count: draft.test_inputs.len(),
        })
        .collect::<Vec<_>>();
    entries.extend(
        extra_static_past_problems()
            .into_iter()
            .map(|problem| HistoryEntry {
                id: problem.id,
                title: problem.title,
                created_at: problem.created_at,
                difficulty: problem.difficulty,
                topic_titles: problem.topic_titles,
                test_count: problem.tests.len(),
            }),
    );
    Ok(entries)
}

fn save_problem_record(app: &tauri::AppHandle, record: &ProblemRecord) -> Result<(), String> {
    let problem_dir = history_directory(app)?.join(&record.id);
    let tests_dir = problem_dir.join("tests");
    fs::create_dir_all(&tests_dir).map_err(|error| format!("无法创建题目历史目录：{error}"))?;
    write_json(&problem_dir.join("problem.json"), &json!(record))?;
    fs::write(problem_dir.join("statement.md"), render_statement(record))
        .map_err(|error| format!("无法写入题面：{error}"))?;
    let extension = extension_for_language(&record.reference_solution.language)?;
    fs::write(
        problem_dir.join(format!("reference.{extension}")),
        &record.reference_solution.code,
    )
    .map_err(|error| format!("无法写入参考代码：{error}"))?;
    for case in record.samples.iter().chain(record.tests.iter()) {
        fs::write(tests_dir.join(format!("{}.in", case.name)), &case.input)
            .map_err(|error| format!("无法写入测试输入：{error}"))?;
        fs::write(
            tests_dir.join(format!("{}.out", case.name)),
            &case.expected_output,
        )
        .map_err(|error| format!("无法写入测试输出：{error}"))?;
    }

    let mut history = read_history_index(app)?;
    history.retain(|item| item.id != record.id);
    history.insert(
        0,
        HistoryEntry {
            id: record.id.clone(),
            title: record.title.clone(),
            created_at: record.created_at.clone(),
            difficulty: record.difficulty.clone(),
            topic_titles: record.topic_titles.clone(),
            test_count: record.tests.len(),
        },
    );
    save_history_index(app, &history)
}

fn render_statement(record: &ProblemRecord) -> String {
    let constraints = record
        .constraints
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    let samples = record
        .samples
        .iter()
        .map(|case| {
            format!(
                "### {}\n\n输入：\n```text\n{}\n```\n\n输出：\n```text\n{}\n```",
                case.name, case.input, case.expected_output
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "# {}\n\n{}\n\n## 输入形式\n\n{}\n\n## 输出形式\n\n{}\n\n## 约束\n\n{}\n\n## 样例\n\n{}\n",
        record.title,
        record.statement,
        record.input_format,
        record.output_format,
        constraints,
        samples
    )
}

fn load_problem_record(app: &tauri::AppHandle, problem_id: &str) -> Result<ProblemRecord, String> {
    if let Some(problem) = builtin_past_problem(app, problem_id)? {
        return Ok(problem);
    }
    let path = history_directory(app)?
        .join(problem_id)
        .join("problem.json");
    let content = fs::read_to_string(path).map_err(|error| format!("无法读取题目历史：{error}"))?;
    let mut record: ProblemRecord =
        serde_json::from_str(&content).map_err(|error| format!("题目历史格式错误：{error}"))?;
    normalize_problem_record_text(&mut record);
    Ok(record)
}

fn delete_problem_record(
    app: &tauri::AppHandle,
    problem_id: &str,
) -> Result<Vec<HistoryEntry>, String> {
    if problem_id.starts_with("past-") {
        return Err("内置历年题不能删除".to_string());
    }
    let mut history = read_history_index(app)?;
    history.retain(|item| item.id != problem_id);

    let problem_dir = history_directory(app)?.join(problem_id);
    match fs::remove_dir_all(&problem_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("无法删除题目文件：{error}")),
    }
    save_history_index(app, &history)?;
    Ok(history)
}

#[tauri::command]
fn bootstrap(app: tauri::AppHandle) -> Result<BootstrapData, String> {
    let data_dir = app_data_directory(&app)?;
    fs::create_dir_all(&data_dir).map_err(|error| format!("无法创建数据目录：{error}"))?;
    fs::create_dir_all(history_directory(&app)?)
        .map_err(|error| format!("无法创建历史目录：{error}"))?;
    fs::create_dir_all(cache_directory(&app)?)
        .map_err(|error| format!("无法创建 Agent 缓存目录：{error}"))?;
    fs::create_dir_all(builtin_problem_directory(&app)?)
        .map_err(|error| format!("无法创建内置历年题缓存目录：{error}"))?;
    Ok(BootstrapData {
        topics: collect_topics(&app),
        past_problems: past_problem_index()?,
        history: read_history_index(&app)?,
        data_directory: data_dir.display().to_string(),
        settings: read_settings(&app)?,
    })
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<AppSettings, String> {
    save_settings_file(&app, &settings)?;
    Ok(settings)
}

#[tauri::command]
async fn generate_problem(
    app: tauri::AppHandle,
    request: GenerateRequest,
) -> Result<ProblemRecord, String> {
    let request = hydrate_request_from_settings(&app, request)?;
    save_settings_file(
        &app,
        &AppSettings {
            api_key: request.api_key.clone(),
            api_url: request.api_url.clone(),
            model: request.model.clone(),
            use_cache: request.use_cache,
        },
    )?;
    let draft = call_generation_api(&app, &request).await?;
    let record = build_problem_record(&request, draft)?;
    save_problem_record(&app, &record)?;
    Ok(record)
}

#[tauri::command]
fn load_problem(app: tauri::AppHandle, problem_id: String) -> Result<ProblemRecord, String> {
    load_problem_record(&app, &problem_id)
}

#[tauri::command]
fn delete_problem(app: tauri::AppHandle, problem_id: String) -> Result<Vec<HistoryEntry>, String> {
    delete_problem_record(&app, &problem_id)
}

#[tauri::command]
fn open_test_file(
    app: tauri::AppHandle,
    problem_id: String,
    case_name: String,
    file_name: String,
) -> Result<(), String> {
    let record = load_problem_record(&app, &problem_id)?;
    let case = record
        .samples
        .iter()
        .chain(record.tests.iter())
        .find(|case| case.name == case_name)
        .ok_or_else(|| format!("未找到用例：{case_name}"))?;
    let file = case
        .files
        .iter()
        .find(|file| file.name == file_name)
        .ok_or_else(|| format!("用例 {case_name} 没有附件文件：{file_name}"))?;
    let file_dir = case_files_directory(&app)?
        .join(safe_file_stem(&problem_id))
        .join(safe_file_stem(&case_name));
    fs::create_dir_all(&file_dir).map_err(|error| format!("无法创建附件目录：{error}"))?;
    let path = write_support_file(&file_dir, file)?;
    let base = fs::canonicalize(case_files_directory(&app)?)
        .map_err(|error| format!("无法定位附件目录：{error}"))?;
    let target = fs::canonicalize(&path).map_err(|error| format!("无法定位附件文件：{error}"))?;
    if !target.starts_with(&base) {
        return Err("只能打开本应用生成的样例附件".to_string());
    }
    open_filesystem_path(&target)
}

#[tauri::command]
fn judge_submission(app: tauri::AppHandle, request: JudgeRequest) -> Result<JudgeResult, String> {
    if !matches!(request.language.as_str(), "c11" | "c") {
        return Err("当前评测只支持 C 语言提交".to_string());
    }
    let record = load_problem_record(&app, &request.problem_id)?;
    let compile_started = Instant::now();
    let compiled = compile_source(&request.language, &request.code, "auto-judge-user")?;
    let compile_elapsed_ms = compile_started.elapsed().as_millis();
    let compiled = match compiled {
        Ok(program) => program,
        Err((stdout, stderr)) => {
            return Ok(JudgeResult {
                status: "CE".to_string(),
                passed: 0,
                total: record.tests.len(),
                compile_elapsed_ms,
                run_elapsed_ms: 0,
                compile_stdout: stdout,
                compile_stderr: stderr,
                cases: Vec::new(),
            });
        }
    };

    let mut results = Vec::new();
    let run_dir =
        judge_runs_directory(&app)?.join(format!("{}-{}", request.problem_id, now_millis()?));
    fs::create_dir_all(&run_dir).map_err(|error| format!("无法创建评测结果目录：{error}"))?;
    for case in &record.tests {
        let output = run_program(&compiled, &case.input, &record.io_mode, &case.files)?;
        let status = if output.status == "OK" {
            if normalize_output(&output.stdout) == normalize_output(&case.expected_output) {
                "AC"
            } else {
                "WA"
            }
        } else {
            output.status.as_str()
        }
        .to_string();
        let artifact_path = write_case_artifact(
            &run_dir,
            case,
            &status,
            output.elapsed_ms,
            &output.stdout,
            &output.stderr,
        )?;
        results.push(CaseResult {
            name: case.name.clone(),
            status,
            elapsed_ms: output.elapsed_ms,
            expected_output: String::new(),
            actual_output: String::new(),
            stderr: String::new(),
            artifact_path: artifact_path.to_string_lossy().to_string(),
        });
    }
    let _ = fs::remove_dir_all(&compiled.work_dir);
    let passed = results.iter().filter(|item| item.status == "AC").count();
    let total = results.len();
    let run_elapsed_ms = results.iter().map(|item| item.elapsed_ms).sum();
    let status = if passed == total { "AC" } else { "WA" }.to_string();
    Ok(JudgeResult {
        status,
        passed,
        total,
        compile_elapsed_ms,
        run_elapsed_ms,
        compile_stdout: String::new(),
        compile_stderr: String::new(),
        cases: results,
    })
}

#[tauri::command]
fn open_case_artifact(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let base = judge_runs_directory(&app)?;
    let base = fs::canonicalize(&base).map_err(|error| format!("无法定位评测结果目录：{error}"))?;
    let target = fs::canonicalize(&path).map_err(|error| format!("无法定位评测文件：{error}"))?;
    if !target.starts_with(&base) {
        return Err("只能打开本应用生成的评测结果文件夹".to_string());
    }
    if !target.is_dir() {
        return Err("评测结果路径不是文件夹".to_string());
    }
    open_filesystem_path(&target)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            save_settings,
            generate_problem,
            load_problem,
            delete_problem,
            open_test_file,
            judge_submission,
            open_case_artifact
        ])
        .run(tauri::generate_context!())
        .expect("failed to run tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    #[test]
    fn normalize_output_ignores_spacing_noise() {
        assert_eq!(
            normalize_output("1  2\r\n3\n"),
            normalize_output("1 2 3\n\n")
        );
    }

    #[test]
    fn model_text_newline_escapes_are_decoded_before_display() {
        let code = "#include <stdio.h>\nint main(void){printf(\"%d\\n\", 1);return 0;}\n";
        let mut draft = GeneratedProblemDraft {
            title: "路径整理".to_string(),
            difficulty: "easy".to_string(),
            statement: "第一段规则\\n第二段规则\\r\\n第三段规则".to_string(),
            input_format: "第一行 n\\n随后 n 行记录。".to_string(),
            output_format: "按要求输出。".to_string(),
            constraints: vec!["1 <= n <= 10\\n路径可能包含 Windows 盘符。".to_string()],
            tags: vec!["字符串".to_string()],
            io_mode: IoMode {
                kind: "stdio".to_string(),
                input_file: None,
                output_file: None,
            },
            samples: vec![SampleCase {
                input: "2\\nD:\\new.txt\\nD:\\root.txt\\n".to_string(),
                output: Some("D:\\new.txt\\n".to_string()),
            }],
            reference_solution: ReferenceSolution {
                language: "c11".to_string(),
                code: code.to_string(),
            },
            data_generator: None,
            test_inputs: vec!["1\\nD:\\note.txt\\n".to_string()],
        };

        normalize_generated_draft_text(&mut draft);

        assert_eq!(draft.statement, "第一段规则\n第二段规则\n第三段规则");
        assert_eq!(draft.input_format, "第一行 n\n随后 n 行记录。");
        assert_eq!(
            draft.constraints[0],
            "1 <= n <= 10\n路径可能包含 Windows 盘符。"
        );
        assert_eq!(draft.samples[0].input, "2\nD:\\new.txt\nD:\\root.txt\n");
        assert_eq!(draft.samples[0].output.as_deref(), Some("D:\\new.txt\n"));
        assert_eq!(draft.test_inputs[0], "1\nD:\\note.txt\n");
        assert_eq!(draft.reference_solution.code, code);
    }

    #[test]
    fn case_artifact_files_are_written_to_case_directory() {
        let run_dir = std::env::temp_dir().join(format!(
            "auto-judge-artifact-test-{}",
            now_millis().unwrap()
        ));
        let case = TestCase {
            name: "large-01".to_string(),
            input: "1 2\n".to_string(),
            expected_output: "3\n".to_string(),
            files: vec![TestFile {
                name: "in.txt".to_string(),
                content: "support\n".to_string(),
            }],
        };
        let artifact = write_case_artifact(&run_dir, &case, "WA", 7, "4\n", "trace\n")
            .expect("artifact should be written");
        assert_eq!(
            fs::read_to_string(artifact.join("input.txt")).unwrap(),
            "1 2\n"
        );
        assert_eq!(
            fs::read_to_string(artifact.join("expected.txt")).unwrap(),
            "3\n"
        );
        assert_eq!(
            fs::read_to_string(artifact.join("actual.txt")).unwrap(),
            "4\n"
        );
        assert_eq!(
            fs::read_to_string(artifact.join("stderr.txt")).unwrap(),
            "trace\n"
        );
        assert_eq!(
            fs::read_to_string(artifact.join("in.txt")).unwrap(),
            "support\n"
        );
        assert!(fs::read_to_string(artifact.join("summary.txt"))
            .unwrap()
            .contains("status: WA"));
        let _ = fs::remove_dir_all(run_dir);
    }

    #[test]
    fn generated_test_inputs_are_split_from_generator_output() {
        let output = "1 2\n---AUTO_JUDGE_CASE---\n3 4\n\n---AUTO_JUDGE_CASE---\n5 6";
        let cases = parse_generated_test_inputs(output);
        assert_eq!(cases, vec!["1 2\n", "3 4\n", "5 6\n"]);
    }

    #[test]
    fn generated_test_inputs_must_have_variety_and_large_half() {
        let identical = vec!["1 2\n".to_string(); 10];
        assert!(validate_generated_test_inputs(&identical).is_err());

        let flat = (0..10)
            .map(|index| format!("{index} {}\n", index + 1))
            .collect::<Vec<_>>();
        assert!(validate_generated_test_inputs(&flat).is_err());

        let mut scaled = (0..5)
            .map(|index| format!("{}\n", index + 1))
            .collect::<Vec<_>>();
        scaled.extend((0..5).map(|index| {
            let rows = (0..(30 + index * 5))
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            format!("{}\n{}\n", 30 + index * 5, rows)
        }));
        validate_generated_test_inputs(&scaled).expect("scaled generated data should pass");
    }

    #[test]
    fn problem_record_uses_local_generator_for_formal_tests() {
        let generator_code = r#"#include <stdio.h>
int main(void) {
    for (int i = 0; i < 10; ++i) {
        if (i) puts("---AUTO_JUDGE_CASE---");
        int n = i < 5 ? i + 1 : 30 + (i - 5) * 5;
        printf("%d\n", n);
        for (int j = 0; j < n; ++j) {
            if (j) putchar(' ');
            printf("%d", 1);
        }
        putchar('\n');
    }
    return 0;
}
"#;
        let draft = GeneratedProblemDraft {
            title: "序列求和".to_string(),
            difficulty: "easy".to_string(),
            statement: "输入一个整数序列，输出和。".to_string(),
            input_format: "第一行 n，第二行 n 个整数。".to_string(),
            output_format: "一个整数。".to_string(),
            constraints: vec!["1 <= n <= 50".to_string()],
            tags: vec!["基础".to_string()],
            io_mode: IoMode {
                kind: "stdio".to_string(),
                input_file: None,
                output_file: None,
            },
            samples: vec![
                SampleCase {
                    input: "3\n1 2 3\n".to_string(),
                    output: None,
                },
                SampleCase {
                    input: "2\n4 5\n".to_string(),
                    output: None,
                },
            ],
            reference_solution: ReferenceSolution {
                language: "c11".to_string(),
                code: "#include <stdio.h>\nint main(void){int n,x,sum=0;if(scanf(\"%d\",&n)!=1)return 0;for(int i=0;i<n;i++){scanf(\"%d\",&x);sum+=x;}printf(\"%d\\n\",sum);return 0;}\n".to_string(),
            },
            data_generator: Some(DataGenerator {
                language: "c11".to_string(),
                code: generator_code.to_string(),
            }),
            test_inputs: Vec::new(),
        };

        let record = problem_record_from_draft(
            "generated-sum".to_string(),
            "now".to_string(),
            vec!["基础".to_string()],
            draft,
        )
        .expect("generator-backed problem should build");
        assert_eq!(record.tests.len(), 10);
        assert_eq!(record.tests[0].input, "1\n1\n");
        assert_eq!(record.tests[0].expected_output, "1\n");
        assert!(record.tests[9].input.starts_with("50\n"));
        assert_eq!(record.tests[9].expected_output, "50\n");
    }

    #[test]
    fn deepseek_base_url_is_normalized_to_chat_completions() {
        assert_eq!(
            chat_completions_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://api.deepseek.com/v1/chat/completions"),
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://api.deepseek.com"),
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://example.com/v1"),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn courseware_topics_are_curated_knowledge_points() {
        let topics = collect_courseware_topics();
        assert!(!topics.is_empty());
        assert!(topics.len() >= knowledge_specs().len());
        assert!(topics.iter().all(|topic| topic.source == "knowledge"));
        assert!(topics.iter().all(|topic| topic.source != "ds_lz"));
        assert!(topics.iter().all(|topic| !topic.title.contains("微信图片")));
        assert!(topics.iter().all(|topic| !topic.path.ends_with(".png")));
        let sorting = topics
            .iter()
            .find(|topic| topic.id == "knowledge-sorting")
            .expect("sorting PPTs in ds_lz should be curated as a knowledge point");
        assert!(sorting.title.contains("冒泡"));
        assert!(sorting.title.contains("归并"));
        assert!(sorting.path.contains("[14]"));
        assert!(sorting.path.contains("[15]"));
    }

    #[test]
    fn builtin_past_problems_are_judge_ready() {
        let problems = builtin_past_problems().expect("builtin past problems should build");
        assert_eq!(problems.len(), 12);
        let ids = problems
            .iter()
            .map(|problem| problem.id.as_str())
            .collect::<Vec<_>>();
        for expected_id in [
            "past-2018-student-online-time",
            "past-2018-postfix-expression",
            "past-2018-network-printer",
            "past-2019-memory-block-merge",
            "past-2019-train-dispatch",
            "past-2019-find-same-file",
            "past-2020-exam-login-anomaly",
            "past-2021-hamming-distance",
            "past-2021-binary-search-tree",
            "past-2021-mini-interpreter",
            "past-2022-co-location",
            "past-2022-file-copy",
        ] {
            assert!(ids.contains(&expected_id), "missing {expected_id}");
        }
        for problem in problems {
            assert!(problem.id.starts_with("past-"));
            assert_eq!(problem.tests.len(), 10);
            assert_eq!(problem.tests[0].name, "small-01");
            assert_eq!(problem.tests[4].name, "small-05");
            assert_eq!(problem.tests[5].name, "large-01");
            assert_eq!(problem.tests[9].name, "large-05");
            assert!(!problem.samples.is_empty());
            assert!(problem
                .samples
                .iter()
                .all(|case| case.input.lines().count() <= 12
                    && case.expected_output.lines().count() <= 12));
            assert!(problem
                .tests
                .iter()
                .all(|case| !case.input.trim().is_empty()));
        }
    }

    #[test]
    fn stale_builtin_cache_with_small_large_cases_is_rejected() {
        let mut record = ProblemRecord {
            id: "past-2018-student-online-time".to_string(),
            created_at: "builtin".to_string(),
            title: "学生在线上机时间统计".to_string(),
            difficulty: "medium".to_string(),
            statement: String::new(),
            input_format: String::new(),
            output_format: String::new(),
            constraints: Vec::new(),
            tags: Vec::new(),
            topic_titles: Vec::new(),
            io_mode: IoMode {
                kind: "stdio".to_string(),
                input_file: None,
                output_file: None,
            },
            samples: Vec::new(),
            tests: (0..10)
                .map(|index| TestCase {
                    name: format!("test-{index:02}"),
                    input: "12\n".to_string(),
                    expected_output: String::new(),
                    files: Vec::new(),
                })
                .collect(),
            reference_solution: ReferenceSolution {
                language: "c11".to_string(),
                code: String::new(),
            },
        };
        apply_scaled_test_names(&mut record);
        assert!(!builtin_cache_is_current(&record));
        record.tests[9].input = student_online_input(100);
        assert!(builtin_cache_is_current(&record));
    }

    #[tokio::test]
    async fn request_generation_draft_uses_user_api_url_and_key() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock server should bind");
        let address = listener.local_addr().expect("mock server address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("mock server should accept request");
            let mut buffer = [0_u8; 8192];
            let bytes = stream
                .read(&mut buffer)
                .expect("mock server should read request");
            let request_text = String::from_utf8_lossy(&buffer[..bytes]).to_string();
            assert!(request_text
                .to_ascii_lowercase()
                .contains("authorization: bearer test-key"));
            assert!(request_text.contains("\"model\":\"test-model\""));
            assert!(request_text.contains("\"max_tokens\":32000"));

            let draft = json!({
                "title": "两数求和",
                "difficulty": "easy",
                "statement": "输入两个整数，输出它们的和。",
                "inputFormat": "一行两个整数。",
                "outputFormat": "一个整数。",
                "constraints": ["0 <= a,b <= 100"],
                "tags": ["基础"],
                "ioMode": { "kind": "stdio", "inputFile": null, "outputFile": null },
                "samples": [
                    { "input": "1 2\n", "output": "" },
                    { "input": "100 200\n", "output": "" }
                ],
                "referenceSolution": {
                    "language": "c11",
                    "code": "#include <stdio.h>\nint main(void){int a,b;if(scanf(\"%d%d\",&a,&b)!=2)return 0;printf(\"%d\\n\",a+b);return 0;}\n"
                },
                "testInputs": ["0 0\n", "1 2\n", "2 3\n", "3 4\n", "4 5\n", "5 6\n", "6 7\n", "7 8\n", "8 9\n", "9 10\n"]
            });
            let body = json!({
                "choices": [
                    {
                        "message": {
                            "content": draft.to_string()
                        }
                    }
                ]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("mock server should write response");
        });

        let request = GenerateRequest {
            api_key: "test-key".to_string(),
            api_url: format!("http://{address}/v1/chat/completions"),
            model: "test-model".to_string(),
            use_cache: false,
            topics: vec![Topic {
                id: "knowledge-test".to_string(),
                title: "顺序表".to_string(),
                source: "knowledge".to_string(),
                year: None,
                path: String::new(),
                excerpt: "测试考点".to_string(),
            }],
            difficulty: "medium".to_string(),
            include_file_io: false,
            extra_requirements: String::new(),
        };
        let draft = request_generation_draft(&request, "生成一道题")
            .await
            .expect("mock response should parse");
        handle.join().expect("mock server thread should finish");

        assert_eq!(draft.title, "两数求和");
        assert_eq!(draft.test_inputs.len(), 10);
        assert_eq!(draft.reference_solution.language, "c11");
    }

    #[test]
    fn compiles_and_runs_cpp17_with_stdio() {
        let source = r#"
#include <iostream>
using namespace std;
int main() {
    int a, b;
    cin >> a >> b;
    cout << a + b << "\n";
    return 0;
}
"#;
        let compiled = compile_source("cpp17", source, "auto-judge-test")
            .expect("compile command should run")
            .expect("source should compile");
        let output = run_program(
            &compiled,
            "19 23\n",
            &IoMode {
                kind: "stdio".to_string(),
                input_file: None,
                output_file: None,
            },
            &[],
        )
        .expect("program should run");
        let _ = fs::remove_dir_all(&compiled.work_dir);
        assert_eq!(output.status, "OK");
        assert_eq!(normalize_output(&output.stdout), "42");
    }

    #[test]
    fn compiles_and_runs_c11_with_file_io() {
        let source = r#"
#include <stdio.h>
int main(void) {
    freopen("data.in", "r", stdin);
    freopen("data.out", "w", stdout);
    int a, b;
    if (scanf("%d%d", &a, &b) != 2) return 1;
    printf("%d\n", a * b);
    return 0;
}
"#;
        let compiled = compile_source("c11", source, "auto-judge-test")
            .expect("compile command should run")
            .expect("source should compile");
        let output = run_program(
            &compiled,
            "7 8\n",
            &IoMode {
                kind: "file".to_string(),
                input_file: Some("data.in".to_string()),
                output_file: Some("data.out".to_string()),
            },
            &[],
        )
        .expect("program should run");
        let _ = fs::remove_dir_all(&compiled.work_dir);
        assert_eq!(output.status, "OK");
        assert_eq!(normalize_output(&output.stdout), "56");
    }
}
