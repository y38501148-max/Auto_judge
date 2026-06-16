export interface Topic {
  id: string
  title: string
  source: string
  year?: string | null
  path: string
  excerpt: string
}

export interface HistoryEntry {
  id: string
  title: string
  createdAt: string
  difficulty: string
  topicTitles: string[]
  testCount: number
}

export interface AppSettings {
  apiKey: string
  apiUrl: string
  model: string
  useCache: boolean
}

export interface BootstrapData {
  topics: Topic[]
  pastProblems: HistoryEntry[]
  history: HistoryEntry[]
  dataDirectory: string
  settings: AppSettings
}

export interface IoMode {
  kind: 'stdio' | 'file' | string
  inputFile?: string | null
  outputFile?: string | null
}

export interface ReferenceSolution {
  language: string
  code: string
}

export interface TestCase {
  name: string
  input: string
  expectedOutput: string
  files?: TestFile[]
}

export interface TestFile {
  name: string
  content: string
}

export interface ProblemRecord {
  id: string
  createdAt: string
  title: string
  difficulty: string
  statement: string
  inputFormat: string
  outputFormat: string
  constraints: string[]
  tags: string[]
  topicTitles: string[]
  ioMode: IoMode
  samples: TestCase[]
  tests: TestCase[]
  referenceSolution: ReferenceSolution
}

export interface GenerateRequest {
  apiKey: string
  apiUrl: string
  model: string
  useCache: boolean
  topics: Topic[]
  difficulty: string
  includeFileIo: boolean
  extraRequirements: string
}

export interface CaseResult {
  name: string
  status: string
  elapsedMs: number
  expectedOutput: string
  actualOutput: string
  stderr: string
  artifactPath: string
}

export interface JudgeResult {
  status: string
  passed: number
  total: number
  compileElapsedMs: number
  runElapsedMs: number
  compileStdout: string
  compileStderr: string
  cases: CaseResult[]
}
