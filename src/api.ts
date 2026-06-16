import { invoke } from '@tauri-apps/api/core'
import type {
  AppSettings,
  BootstrapData,
  GenerateRequest,
  HistoryEntry,
  JudgeResult,
  ProblemRecord,
  Topic,
} from './types'

const SETTINGS_KEY = 'auto-judge:settings'
const HISTORY_KEY = 'auto-judge:history'
const PROBLEM_PREFIX = 'auto-judge:problem:'

function hasTauriRuntime() {
  return Boolean(window.__TAURI__ || window.__TAURI_INTERNALS__)
}

function fallbackTopics(): Topic[] {
  return [
    {
      id: 'preview-list',
      title: '线性表、链表、查找与插入',
      source: 'knowledge',
      path: '~/Desktop/Coding/DS_helper/resources',
      excerpt: '顺序表和单链表及其查找和插入操作',
    },
    {
      id: 'preview-stack',
      title: '栈与队列综合应用',
      source: 'knowledge',
      path: '~/Desktop/Coding/DS_helper/resources',
      excerpt: '栈、队列、循环队列、表达式求值',
    },
    {
      id: 'preview-graph',
      title: '图遍历、最小生成树、最短路径',
      source: 'knowledge',
      path: '~/Desktop/Coding/DS_helper/resources',
      excerpt: 'DFS、BFS、Prim、Dijkstra',
    },
  ]
}

function fallbackSettings(): AppSettings {
  const saved = localStorage.getItem(SETTINGS_KEY)
  return saved
    ? (JSON.parse(saved) as AppSettings)
    : { apiKey: '', apiUrl: '', model: 'deepseek-v4-pro', useCache: true }
}

function fallbackHistory(): HistoryEntry[] {
  const saved = localStorage.getItem(HISTORY_KEY)
  return saved ? (JSON.parse(saved) as HistoryEntry[]) : []
}

export function bootstrap(): Promise<BootstrapData> {
  if (!hasTauriRuntime()) {
    return Promise.resolve({
      topics: fallbackTopics(),
      pastProblems: [],
      history: fallbackHistory(),
      dataDirectory: 'browser-preview',
      settings: fallbackSettings(),
    })
  }
  return invoke('bootstrap')
}

export function saveSettings(settings: AppSettings): Promise<AppSettings> {
  if (!hasTauriRuntime()) {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings))
    return Promise.resolve(settings)
  }
  return invoke('save_settings', { settings })
}

export function generateProblem(request: GenerateRequest): Promise<ProblemRecord> {
  if (!hasTauriRuntime()) {
    void request
    return Promise.reject(new Error('浏览器预览不运行本地 Agent，请使用桌面端。'))
  }
  return invoke('generate_problem', { request })
}

export function loadProblem(problemId: string): Promise<ProblemRecord> {
  if (!hasTauriRuntime()) {
    const saved = localStorage.getItem(`${PROBLEM_PREFIX}${problemId}`)
    if (!saved) return Promise.reject(new Error('浏览器预览没有该历史题目。'))
    return Promise.resolve(JSON.parse(saved) as ProblemRecord)
  }
  return invoke('load_problem', { problemId })
}

export function deleteProblem(problemId: string): Promise<HistoryEntry[]> {
  if (!hasTauriRuntime()) {
    const history = fallbackHistory().filter((entry) => entry.id !== problemId)
    localStorage.setItem(HISTORY_KEY, JSON.stringify(history))
    localStorage.removeItem(`${PROBLEM_PREFIX}${problemId}`)
    return Promise.resolve(history)
  }
  return invoke('delete_problem', { problemId })
}

export function judgeSubmission(problemId: string, language: string, code: string): Promise<JudgeResult> {
  if (!hasTauriRuntime()) {
    void problemId
    void language
    void code
    return Promise.reject(new Error('浏览器预览不运行本地 Judge，请使用桌面端。'))
  }
  return invoke('judge_submission', { request: { problemId, language, code } })
}

export function openCaseArtifact(path: string): Promise<void> {
  if (!hasTauriRuntime()) {
    void path
    return Promise.reject(new Error('浏览器预览不能打开本地评测文件，请使用桌面端。'))
  }
  return invoke('open_case_artifact', { path })
}

export function openTestFile(problemId: string, caseName: string, fileName: string): Promise<void> {
  if (!hasTauriRuntime()) {
    void problemId
    void caseName
    void fileName
    return Promise.reject(new Error('浏览器预览不能打开本地样例附件，请使用桌面端。'))
  }
  return invoke('open_test_file', { problemId, caseName, fileName })
}
