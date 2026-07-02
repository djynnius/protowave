// REST client. Session rides on the httpOnly cookie (same-origin).

export interface WaveDigest {
  wave: string
  title: string
  participants: string[]
  rootWavelet: string
  createdBy: string
  lastActivityMs: number
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
}
