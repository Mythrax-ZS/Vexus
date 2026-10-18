// The only module that talks to the Tauri backend. Everything else goes through these wrappers,
// so errors are normalised once and the UI can run (with a clear message) outside the app.

const T = window.__TAURI__;

export const inTauri = !!(T && T.core && T.core.invoke);

/** A user-presentable error from the backend (`{ code, title, detail }`). */
export class AppError extends Error {
  constructor({ code = 'internal', title = 'Something went wrong', detail = '' } = {}) {
    super(detail || title);
    this.name = 'AppError';
    this.code = code;
    this.title = title;
    this.detail = detail;
  }
}

function normalize(e) {
  if (e instanceof AppError) return e;
  if (e && typeof e === 'object' && 'title' in e && 'code' in e) return new AppError(e);
  const detail = typeof e === 'string' ? e : e?.message ?? String(e);
  return new AppError({ code: 'internal', title: 'Something went wrong', detail });
}

export async function invoke(command, args) {
  if (!inTauri) {
    throw new AppError({
      code: 'no-backend',
      title: 'Open Vexus from the desktop app',
      detail: 'This page needs the Vexus engine, which only runs inside the desktop app.',
    });
  }
  try {
    return await T.core.invoke(command, args);
  } catch (e) {
    throw normalize(e);
  }
}

/** A streaming channel from Rust; `handler` receives each event in order. */
export function channel(handler) {
  const ch = new T.core.Channel();
  ch.onmessage = handler;
  return ch;
}

export function onAppEvent(name, handler) {
  if (!inTauri) return () => {};
  const p = T.event.listen(name, (e) => handler(e.payload));
  return () => p.then((unlisten) => unlisten()).catch(() => {});
}

export function appWindow() {
  return inTauri ? T.window.getCurrentWindow() : null;
}
