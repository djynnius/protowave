// REST client. Session rides on the httpOnly cookie (same-origin).

export interface WaveDigest {
  wave: string
  title: string
  participants: string[]
  rootWavelet: string
  createdBy: string
  lastActivityMs: number
  unread: boolean
  unreadCount: number
  translationEnabled: boolean
  archived?: boolean
}

export interface AttachmentMeta {
  hash: string
  wave: string
  name: string
  mime: string
  size: number
  uploader: string
}

export interface SearchHit {
  wave: string
  title: string
  snippet: string
}

export interface PoolModel {
  id: string
  owner: string
  ownerName: string
  label: string
  base: string
  model: string
  scope: 'private' | 'wave' | 'federation'
  enabled: boolean
  mine: boolean
}

export interface ShareMeta {
  manifest_hash: string
  wave: string
  name: string
  total_size: number
  file_count: number
  uploader: string
  origin_domain: string
  mirrored: boolean
}

export interface ShareManifest {
  name: string
  totalSize: number
  files: { path: string; size: number; mime: string; chunks: number }[]
}

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message)
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    credentials: 'same-origin',
    headers: init?.body ? { 'Content-Type': 'application/json' } : undefined,
    ...init,
  })
  const body = res.status === 204 ? null : await res.json().catch(() => null)
  if (!res.ok) {
    throw new ApiError(res.status, body?.error ?? res.statusText)
  }
  return body as T
}

export const api = {
  register: (name: string, password: string) =>
    request<{ participant: string }>('/api/register', {
      method: 'POST',
      body: JSON.stringify({ name, password }),
    }),
  login: (name: string, password: string) =>
    request<{ participant: string }>('/api/login', {
      method: 'POST',
      body: JSON.stringify({ name, password }),
    }),
  logout: () => request<{ ok: boolean }>('/api/logout', { method: 'POST' }),
  changePassword: (currentPassword: string, newPassword: string) =>
    request<{ ok: boolean }>('/api/password', {
      method: 'POST',
      body: JSON.stringify({ currentPassword, newPassword }),
    }),
  me: () =>
    request<{ participant: string; firstName?: string; lastName?: string; isOwner?: boolean }>(
      '/api/me',
    ),
  listWaves: () => request<WaveDigest[]>('/api/waves'),
  createWave: (title: string) =>
    request<WaveDigest>('/api/waves', { method: 'POST', body: JSON.stringify({ title }) }),
  addParticipant: (wave: string, participant: string) =>
    request<WaveDigest>('/api/waves/participants', {
      method: 'POST',
      body: JSON.stringify({ wave, participant }),
    }),
  archiveWave: (wave: string, archived: boolean) =>
    request<{ ok: boolean }>('/api/waves/archive', {
      method: 'POST',
      body: JSON.stringify({ wave, archived }),
    }),
  deleteWave: (wave: string) =>
    request<{ ok: boolean }>('/api/waves/delete', {
      method: 'POST',
      body: JSON.stringify({ wave }),
    }),
  setProfile: (firstName: string, lastName: string) =>
    request<{ firstName: string; lastName: string }>('/api/profile', {
      method: 'POST',
      body: JSON.stringify({ firstName, lastName }),
    }),
  userProfile: (participant: string) =>
    request<{
      participant: string
      firstName: string
      lastName: string
      displayName: string
      sharedWaves: { wave: string; title: string }[]
    }>(`/api/users/${encodeURIComponent(participant)}`),
  getSettings: () =>
    request<{
      domain: string
      inferenceProvider: string
      inferenceBase: string
      inferenceModel: string
      activeModel: string
      geminiKeyPresent: boolean
    }>('/api/settings'),
  putSettings: (provider: string, base: string, model: string) =>
    request<{ ok: boolean; activeModel: string }>('/api/settings', {
      method: 'PUT',
      body: JSON.stringify({ provider, base, model }),
    }),
  listModels: () => request<{ mine: PoolModel[]; pool: PoolModel[] }>('/api/models'),
  putModel: (m: {
    id?: string
    label: string
    base: string
    model: string
    scope: string
    enabled?: boolean
  }) => request<PoolModel>('/api/models', { method: 'POST', body: JSON.stringify(m) }),
  deleteModel: (id: string) =>
    request<{ ok: boolean }>(`/api/models/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  testModel: (base: string, model: string) =>
    request<{ ok: boolean; model?: string; error?: string }>('/api/models/test', {
      method: 'POST',
      body: JSON.stringify({ base, model }),
    }),
  ask: (wave: string, prompt: string) =>
    request<{ answer: string; model: string; agent: string }>('/api/waves/ask', {
      method: 'POST',
      body: JSON.stringify({ wave, prompt }),
    }),
  setTranslation: (wave: string, enabled: boolean) =>
    request<WaveDigest>('/api/waves/translation', {
      method: 'POST',
      body: JSON.stringify({ wave, enabled }),
    }),
  markRead: (wave: string) =>
    request<{ ok: boolean }>('/api/waves/read', {
      method: 'POST',
      body: JSON.stringify({ wave }),
    }),
  search: (q: string) => request<SearchHit[]>(`/api/search?q=${encodeURIComponent(q)}`),
  listAttachments: (wave: string) =>
    request<AttachmentMeta[]>(`/api/attachments?wave=${encodeURIComponent(wave)}`),
  listShares: (wave: string) =>
    request<ShareMeta[]>(`/api/shares?wave=${encodeURIComponent(wave)}`),
  shareManifest: (hash: string) => request<ShareManifest>(`/api/shares/${hash}`),
  mirrorShare: (hash: string) =>
    request<ShareMeta>(`/api/shares/${hash}/mirror`, { method: 'POST' }),
  uploadFolder: async (wave: string, name: string, files: File[]): Promise<ShareMeta> => {
    const form = new FormData()
    for (const file of files) {
      const rel = (file as File & { webkitRelativePath?: string }).webkitRelativePath || file.name
      form.append('file', file, rel)
    }
    const res = await fetch(
      `/api/shares?wave=${encodeURIComponent(wave)}&name=${encodeURIComponent(name)}`,
      { method: 'POST', credentials: 'same-origin', body: form },
    )
    const body = await res.json().catch(() => null)
    if (!res.ok) throw new ApiError(res.status, body?.error ?? res.statusText)
    return body as ShareMeta
  },
  uploadAttachment: async (wave: string, file: File): Promise<AttachmentMeta> => {
    const form = new FormData()
    form.append('file', file, file.name)
    const res = await fetch(`/api/attachments?wave=${encodeURIComponent(wave)}`, {
      method: 'POST',
      credentials: 'same-origin',
      body: form,
    })
    const body = await res.json().catch(() => null)
    if (!res.ok) throw new ApiError(res.status, body?.error ?? res.statusText)
    return body as AttachmentMeta
  },
}
