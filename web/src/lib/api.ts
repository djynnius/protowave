// REST client. Session rides on the httpOnly cookie (same-origin).

export interface WaveDigest {
  wave: string
  title: string
  participants: string[]
  rootWavelet: string
  createdBy: string
  lastActivityMs: number
  unread: boolean
  translationEnabled: boolean
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
  me: () => request<{ participant: string }>('/api/me'),
  listWaves: () => request<WaveDigest[]>('/api/waves'),
  createWave: (title: string) =>
    request<WaveDigest>('/api/waves', { method: 'POST', body: JSON.stringify({ title }) }),
  addParticipant: (wave: string, participant: string) =>
    request<WaveDigest>('/api/waves/participants', {
      method: 'POST',
      body: JSON.stringify({ wave, participant }),
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
