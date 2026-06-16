import {
  ArrowLeft,
  CheckCircle2,
  Copy,
  Database,
  FileCode2,
  FolderClock,
  FolderOpen,
  Loader2,
  Play,
  RefreshCw,
  Save,
  Settings,
  Trash2,
  XCircle,
} from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import {
  bootstrap,
  deleteProblem,
  generateProblem,
  judgeSubmission,
  loadProblem,
  openCaseArtifact,
  openTestFile,
  saveSettings,
} from './api'
import type { AppSettings, BootstrapData, HistoryEntry, JudgeResult, ProblemRecord, Topic } from './types'

const starterCode = `#include <stdio.h>

int main(void) {
    return 0;
}
`

function formatTime(value: string) {
  const numeric = Number(value)
  if (!Number.isFinite(numeric)) return value
  return new Date(numeric).toLocaleString()
}

function difficultyLabel(value: string) {
  if (value === 'easy') return '简单'
  if (value === 'hard') return '困难'
  if (value === 'medium') return '中等'
  return value
}

function difficultyDescription(value: string) {
  if (value === 'easy') return '规则较直观，重点是输入输出、结构体数组、排序和边界。'
  if (value === 'hard') return '题面规则更多，可能包含文件或树状数据，代码量和细节明显增加。'
  return '接近往年机试风格，理解规则、建模和模拟处理并重。'
}

function sourceLabel(value: string) {
  if (value === 'knowledge') return '知识点'
  return value
}

function emptySettings(): AppSettings {
  return {
    apiKey: '',
    apiUrl: '',
    model: 'deepseek-v4-pro',
    useCache: true,
  }
}

export function App() {
  const [boot, setBoot] = useState<BootstrapData | null>(null)
  const [settings, setSettings] = useState<AppSettings>(emptySettings)
  const [selectedTopicIds, setSelectedTopicIds] = useState<string[]>([])
  const [sourceFilter, setSourceFilter] = useState('all')
  const [query, setQuery] = useState('')
  const [includeFileIo, setIncludeFileIo] = useState(false)
  const [difficulty, setDifficulty] = useState('medium')
  const [extraRequirements, setExtraRequirements] = useState('')
  const [activeProblem, setActiveProblem] = useState<ProblemRecord | null>(null)
  const [language] = useState('c11')
  const [code, setCode] = useState(starterCode)
  const [judgeResult, setJudgeResult] = useState<JudgeResult | null>(null)
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const [activeView, setActiveView] = useState<'problem' | 'submit'>('problem')
  const [agentOpen, setAgentOpen] = useState(true)
  const [copiedKey, setCopiedKey] = useState('')

  async function loadApplication() {
    setBusy('bootstrap')
    setError('')
    try {
      const data = await bootstrap()
      setBoot(data)
      setSettings(data.settings)
      if (data.history.length > 0) {
        const latest = await loadProblem(data.history[0].id)
        setActiveProblem(latest)
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy('')
    }
  }

  useEffect(() => {
    void loadApplication()
  }, [])

  const filteredTopics = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase()
    return (boot?.topics ?? []).filter((topic) => {
      const sourceMatches = sourceFilter === 'all' || topic.source === sourceFilter
      const queryMatches =
        normalizedQuery.length === 0 ||
        topic.title.toLowerCase().includes(normalizedQuery) ||
        topic.excerpt.toLowerCase().includes(normalizedQuery)
      return sourceMatches && queryMatches
    })
  }, [boot?.topics, query, sourceFilter])

  const selectedTopics = useMemo(() => {
    const topicById = new Map((boot?.topics ?? []).map((topic) => [topic.id, topic]))
    return selectedTopicIds.map((id) => topicById.get(id)).filter((topic): topic is Topic => Boolean(topic))
  }, [boot?.topics, selectedTopicIds])

  function toggleTopic(topic: Topic) {
    setSelectedTopicIds((current) =>
      current.includes(topic.id) ? current.filter((id) => id !== topic.id) : [...current, topic.id],
    )
  }

  async function handleSaveSettings() {
    setBusy('settings')
    setError('')
    try {
      const saved = await saveSettings(settings)
      setSettings(saved)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy('')
    }
  }

  async function handleGenerate() {
    setBusy('generate')
    setError('')
    setJudgeResult(null)
    try {
      const problem = await generateProblem({
        ...settings,
        topics: selectedTopics,
        difficulty,
        includeFileIo,
        extraRequirements,
      })
      setActiveProblem(problem)
      setActiveView('problem')
      const data = await bootstrap()
      setBoot(data)
      setSettings(data.settings)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy('')
    }
  }

  async function handleLoadHistory(entry: HistoryEntry) {
    setBusy(`history:${entry.id}`)
    setError('')
      setJudgeResult(null)
    try {
      setActiveProblem(await loadProblem(entry.id))
      setActiveView('problem')
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy('')
    }
  }

  async function handleDeleteHistory(entry: HistoryEntry) {
    setBusy(`delete:${entry.id}`)
    setError('')
    try {
      const history = await deleteProblem(entry.id)
      setBoot((current) => (current ? { ...current, history } : current))
      if (activeProblem?.id === entry.id) {
        setActiveProblem(null)
        setJudgeResult(null)
        setActiveView('problem')
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy('')
    }
  }

  async function handleJudge() {
    if (!activeProblem) return
    setBusy('judge')
    setError('')
    try {
      setJudgeResult(await judgeSubmission(activeProblem.id, language, code))
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy('')
    }
  }

  async function handleOpenCaseArtifact(path: string) {
    setError('')
    try {
      await openCaseArtifact(path)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  async function handleOpenTestFile(caseName: string, fileName: string) {
    if (!activeProblem) return
    setError('')
    try {
      await openTestFile(activeProblem.id, caseName, fileName)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  async function handleCopySampleText(key: string, text: string) {
    setError('')
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text)
      } else {
        const textarea = document.createElement('textarea')
        textarea.value = text
        textarea.setAttribute('readonly', 'true')
        textarea.style.position = 'fixed'
        textarea.style.opacity = '0'
        document.body.appendChild(textarea)
        textarea.select()
        document.execCommand('copy')
        document.body.removeChild(textarea)
      }
      setCopiedKey(key)
      window.setTimeout(() => {
        setCopiedKey((current) => (current === key ? '' : current))
      }, 1200)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const isBusy = busy.length > 0

  return (
    <main className={`app-shell ${agentOpen ? '' : 'agent-collapsed'}`}>
      <aside className="sidebar">
        <section className="panel topic-panel">
          <div className="panel-title">
            <Database size={18} />
            <h2>考点</h2>
            <button className="icon-button" onClick={loadApplication} title="刷新" disabled={isBusy}>
              <RefreshCw size={16} />
            </button>
          </div>
          <div className="segmented">
            {['all', 'knowledge'].map((source) => (
              <button
                key={source}
                className={sourceFilter === source ? 'active' : ''}
                onClick={() => setSourceFilter(source)}
              >
                {source === 'all' ? '全部' : sourceLabel(source)}
              </button>
            ))}
          </div>
          <input
            className="text-input"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索"
          />
          <div className="topic-list">
            {filteredTopics.map((topic) => (
              <label key={topic.id} className="topic-row">
                <input
                  type="checkbox"
                  checked={selectedTopicIds.includes(topic.id)}
                  onChange={() => toggleTopic(topic)}
                />
                <span>
                  <strong>{topic.title}</strong>
                  <small>
                    {sourceLabel(topic.source)}
                    {topic.year ? ` · ${topic.year}` : ''}
                  </small>
                </span>
              </label>
            ))}
          </div>
        </section>

        <section className="panel history-panel">
          <div className="panel-title">
            <FolderClock size={18} />
            <h2>题库</h2>
          </div>
          <div className="history-list">
            <div className="list-section-label">内置历年题</div>
            {(boot?.pastProblems ?? []).map((entry) => (
              <button
                key={entry.id}
                className={`history-row ${activeProblem?.id === entry.id ? 'active' : ''}`}
                onClick={() => void handleLoadHistory(entry)}
              >
                <strong>{entry.title}</strong>
                <span>
                  {difficultyLabel(entry.difficulty)} · {entry.testCount} tests
                </span>
                <small>{entry.topicTitles.join(' · ')}</small>
              </button>
            ))}
            <div className="list-section-label">生成历史</div>
            {(boot?.history ?? []).map((entry) => (
              <div key={entry.id} className={`history-row-shell ${activeProblem?.id === entry.id ? 'active' : ''}`}>
                <button className="history-row" onClick={() => void handleLoadHistory(entry)}>
                  <strong>{entry.title}</strong>
                  <span>
                    {difficultyLabel(entry.difficulty)} · {entry.testCount} tests
                  </span>
                  <small>{formatTime(entry.createdAt)}</small>
                </button>
                <button
                  type="button"
                  className="delete-history-button"
                  onClick={(event) => {
                    event.preventDefault()
                    event.stopPropagation()
                    void handleDeleteHistory(entry)
                  }}
                  disabled={busy === `delete:${entry.id}`}
                  aria-label={`删除 ${entry.title}`}
                  title="删除"
                >
                  {busy === `delete:${entry.id}` ? <Loader2 className="spin" size={15} /> : <Trash2 size={15} />}
                </button>
              </div>
            ))}
          </div>
        </section>
      </aside>

      <section className="workspace">
        <header className="topbar">
          <div>
            <h1>Auto Judge</h1>
            <span>{boot?.dataDirectory ?? '...'}</span>
          </div>
          <div className="topbar-actions">
            {error ? <div className="error-banner">{error}</div> : null}
            <button className="icon-text-button" onClick={() => setAgentOpen((current) => !current)}>
              <Settings size={16} />
              {agentOpen ? '关闭 Agent' : 'Agent'}
            </button>
          </div>
        </header>

        {activeView === 'problem' ? (
          <section className="panel problem-panel">
            {activeProblem ? (
            <>
              <div className="problem-heading">
                <div>
                  <h2>{activeProblem.title}</h2>
                  <div className="chips">
                    <span>{difficultyLabel(activeProblem.difficulty)}</span>
                    <span>{activeProblem.ioMode.kind === 'file' ? '文件 I/O' : '标准 I/O'}</span>
                    {activeProblem.tags.map((tag) => (
                      <span key={tag}>{tag}</span>
                    ))}
                  </div>
                </div>
                <div className="problem-actions">
                  <span className="test-count">{activeProblem.tests.length} tests</span>
                  <button className="primary-button" onClick={() => setActiveView('submit')}>
                    <FileCode2 size={16} />
                    提交
                  </button>
                </div>
              </div>
              <article className="statement">
                <p>{activeProblem.statement}</p>
                <h3>输入形式</h3>
                <p>{activeProblem.inputFormat}</p>
                <h3>输出形式</h3>
                <p>{activeProblem.outputFormat}</p>
                <h3>约束</h3>
                <ul>
                  {activeProblem.constraints.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </article>
              <div className="case-grid">
                {activeProblem.samples.map((item) => (
                  <div className="case-box" key={item.name}>
                    <div className="case-box-header">
                      <h3>{item.name}</h3>
                      {item.files?.length ? (
                        <div className="case-file-actions">
                          {item.files.map((file) => (
                            <button
                              key={file.name}
                              type="button"
                              className="open-case-button"
                              onClick={() => void handleOpenTestFile(item.name, file.name)}
                            >
                              <FolderOpen size={14} />
                              打开 {file.name}
                            </button>
                          ))}
                        </div>
                      ) : null}
                    </div>
                    <div className="case-io-title">
                      <label>Input</label>
                      <button
                        type="button"
                        className="copy-case-button"
                        onClick={() => void handleCopySampleText(`${item.name}:input`, item.input)}
                        title="复制输入"
                      >
                        <Copy size={13} />
                        {copiedKey === `${item.name}:input` ? '已复制' : '复制'}
                      </button>
                    </div>
                    <pre>{item.input}</pre>
                    <div className="case-io-title">
                      <label>Output</label>
                      <button
                        type="button"
                        className="copy-case-button"
                        onClick={() => void handleCopySampleText(`${item.name}:output`, item.expectedOutput)}
                        title="复制输出"
                      >
                        <Copy size={13} />
                        {copiedKey === `${item.name}:output` ? '已复制' : '复制'}
                      </button>
                    </div>
                    <pre>{item.expectedOutput}</pre>
                  </div>
                ))}
              </div>
            </>
            ) : (
              <div className="empty-state">选择考点后生成题目</div>
            )}
          </section>
        ) : (
          <section className="panel submit-panel">
            <div className="submit-header">
              <button className="icon-text-button" onClick={() => setActiveView('problem')}>
                <ArrowLeft size={16} />
                返回题目
              </button>
              <div>
                <h2>提交</h2>
                <span>{activeProblem?.title ?? '未选择题目'}</span>
              </div>
              <select value={language} disabled>
                <option value="c11">C11</option>
              </select>
            </div>
            <div className={`submit-body ${judgeResult ? 'has-result' : 'editor-only'}`}>
              <textarea className="submit-editor" value={code} onChange={(event) => setCode(event.target.value)} />
              {judgeResult ? (
                <div className="judge-result">
                  <div className={`result-summary ${judgeResult.status === 'AC' ? 'accepted' : 'failed'}`}>
                    {judgeResult.status === 'AC' ? <CheckCircle2 size={18} /> : <XCircle size={18} />}
                    <strong>{judgeResult.status}</strong>
                    <span>
                      {judgeResult.passed}/{judgeResult.total}
                    </span>
                    <span>运行 {judgeResult.runElapsedMs} ms</span>
                    <span>编译 {judgeResult.compileElapsedMs} ms</span>
                  </div>
                  {judgeResult.compileStderr ? <pre className="compile-error">{judgeResult.compileStderr}</pre> : null}
                  <div className="case-result-list">
                    {judgeResult.cases.map((item) => (
                      <div key={item.name} className={`case-result-row ${item.status === 'AC' ? 'case-pass' : 'case-fail'}`}>
                        <span>{item.name}</span>
                        <strong>{item.status}</strong>
                        <small>{item.elapsedMs} ms</small>
                        <button
                          type="button"
                          className="open-case-button"
                          onClick={() => void handleOpenCaseArtifact(item.artifactPath)}
                        >
                          <FolderOpen size={14} />
                          打开文件
                        </button>
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
            </div>
            <div className="submit-footer">
              <button className="primary-button" onClick={handleJudge} disabled={!activeProblem || isBusy}>
                {busy === 'judge' ? <Loader2 className="spin" size={16} /> : <Play size={16} />}
                提交
              </button>
            </div>
          </section>
        )}
      </section>

      {agentOpen ? (
      <aside className="agent-panel panel">
        <div className="panel-title">
          <Settings size={18} />
          <h2>Agent</h2>
          <button className="icon-text-button" onClick={handleSaveSettings} disabled={isBusy}>
            {busy === 'settings' ? <Loader2 className="spin" size={16} /> : <Save size={16} />}
            保存
          </button>
        </div>
        <label className="field">
          <span>API URL</span>
          <input
            value={settings.apiUrl}
            onChange={(event) => setSettings((current) => ({ ...current, apiUrl: event.target.value }))}
          />
        </label>
        <label className="field">
          <span>API Key</span>
          <input
            type="password"
            value={settings.apiKey}
            onChange={(event) => setSettings((current) => ({ ...current, apiKey: event.target.value }))}
          />
        </label>
        <label className="field">
          <span>Model</span>
          <input
            value={settings.model}
            onChange={(event) => setSettings((current) => ({ ...current, model: event.target.value }))}
          />
        </label>
        <label className="field">
          <span>题目难度</span>
          <select value={difficulty} onChange={(event) => setDifficulty(event.target.value)}>
            <option value="easy">简单</option>
            <option value="medium">中等</option>
            <option value="hard">困难</option>
          </select>
          <small>{difficultyDescription(difficulty)}</small>
        </label>
        <label className="toggle-row">
          <input
            type="checkbox"
            checked={settings.useCache}
            onChange={(event) => setSettings((current) => ({ ...current, useCache: event.target.checked }))}
          />
          <span>Agent 缓存</span>
        </label>
        <label className="toggle-row">
          <input type="checkbox" checked={includeFileIo} onChange={(event) => setIncludeFileIo(event.target.checked)} />
          <span>文件输入输出考点</span>
        </label>
        <label className="field">
          <span>补充要求</span>
          <textarea
            className="requirements"
            value={extraRequirements}
            onChange={(event) => setExtraRequirements(event.target.value)}
          />
        </label>
        <div className="selected-topics">
          {selectedTopics.map((topic) => (
            <span key={topic.id} title={topic.title}>
              {topic.title}
            </span>
          ))}
        </div>
        <button className="generate-button" onClick={handleGenerate} disabled={selectedTopics.length === 0 || isBusy}>
          {busy === 'generate' ? <Loader2 className="spin" size={18} /> : <Play size={18} />}
          生成题目
        </button>
      </aside>
      ) : null}
    </main>
  )
}
