// The simple editor's fields of a menu entry, read from and written into
// its text. Only those keys of the [Desktop Entry] group change; localised
// keys (Name[de]), other keys, groups and comments stay as they are. The
// same rules as forager-core's entries::with_fields.

export interface EntryFields {
  name: string;
  exec: string;
  icon: string;
  comment: string;
  categories: string;
  terminal: boolean;
  wmClass: string;
  noDisplay: boolean;
  /** The folder the app starts in; the home folder when empty. */
  path: string;
}

const MAIN = '[Desktop Entry]';

/** A key's value in the [Desktop Entry] group. */
export function entryField(text: string, key: string): string | undefined {
  let inMain = false;
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (line.startsWith('[')) inMain = line === MAIN;
    else if (inMain) {
      const at = line.indexOf('=');
      if (at > 0 && line.slice(0, at).trim() === key) return line.slice(at + 1).trim();
    }
  }
  return undefined;
}

export function parseFields(text: string): EntryFields {
  const get = (key: string) => entryField(text, key) ?? '';
  return {
    name: get('Name'),
    exec: get('Exec'),
    icon: get('Icon'),
    comment: get('Comment'),
    categories: get('Categories'),
    terminal: get('Terminal') === 'true',
    wmClass: get('StartupWMClass'),
    noDisplay: get('NoDisplay') === 'true',
    path: get('Path'),
  };
}

/** `text` with each key set in the [Desktop Entry] group, or removed for `null`. */
export function setKeys(text: string, keys: [string, string | null][]): string {
  const lineFor = (key: string, value: string) => `${key}=${value.replace(/[\r\n]/g, ' ')}`;
  const out: string[] = [];
  const written = new Set<string>();
  let inMain = false;
  let seenMain = false;
  const flush = () => {
    let at = out.length;
    while (at > 0 && out[at - 1].trim() === '') at--;
    const missing = keys.filter(([k, v]) => v !== null && !written.has(k)).map(([k, v]) => lineFor(k, v!));
    keys.forEach(([k]) => written.add(k));
    out.splice(at, 0, ...missing);
  };
  const lines = text.endsWith('\n') ? text.slice(0, -1).split('\n') : text === '' ? [] : text.split('\n');
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith('[')) {
      if (inMain) flush();
      inMain = trimmed === MAIN;
      seenMain ||= inMain;
      out.push(line);
      continue;
    }
    const at = line.indexOf('=');
    const key = at > 0 ? line.slice(0, at).trim() : null;
    const change = inMain && key !== null ? keys.find(([k]) => k === key) : undefined;
    if (change) {
      if (change[1] !== null && !written.has(change[0])) out.push(lineFor(change[0], change[1]));
      written.add(change[0]);
    } else {
      out.push(line);
    }
  }
  if (inMain) flush();
  if (!seenMain) {
    const head = [MAIN, 'Type=Application', ...keys.filter(([, v]) => v !== null).map(([k, v]) => lineFor(k, v!))];
    if (out.length > 0) head.push('');
    out.unshift(...head);
  }
  return `${out.join('\n')}\n`;
}

/** `text` with `fields` written in; an empty text field removes its key. */
export function withFields(text: string, fields: EntryFields): string {
  const value = (v: string) => (v.trim() === '' ? null : v.trim());
  return setKeys(text, [
    ['Name', value(fields.name)],
    ['Exec', value(fields.exec)],
    ['Icon', value(fields.icon)],
    ['Comment', value(fields.comment)],
    ['Categories', value(fields.categories)],
    ['Terminal', String(fields.terminal)],
    ['StartupWMClass', value(fields.wmClass)],
    ['NoDisplay', fields.noDisplay ? 'true' : null],
    ['Path', value(fields.path)],
  ]);
}

/** Where the program ends in an `Exec` value. */
function programEnd(exec: string): number {
  if (exec.startsWith('"')) {
    // Past the closing quote, skipping escaped ones.
    for (let i = 1; i < exec.length; i++) {
      if (exec[i] === '\\') i++;
      else if (exec[i] === '"') return i + 1;
    }
    return exec.length;
  }
  const space = exec.search(/\s/);
  return space < 0 ? exec.length : space;
}

/** The program an `Exec` value starts, unquoted. */
export function programOf(exec: string): string {
  const e = exec.trim();
  const word = e.slice(0, programEnd(e));
  return word.startsWith('"') ? word.slice(1, word.endsWith('"') ? -1 : undefined).replace(/\\(.)/g, '$1') : word;
}

/** The arguments after the program in an `Exec` value. */
export function argsOf(exec: string): string {
  const e = exec.trim();
  return e.slice(programEnd(e)).trim();
}

/** `program` as one `Exec` argument: quoted when it needs to be, as the specification asks. */
export function quoteProgram(program: string): string {
  if (program === '' || /^[\w./+-]+$/.test(program)) return program;
  return `"${program.replace(/["`$\\]/g, '\\$&')}"`;
}

/** The `Exec` value starting `program` with `args`. */
export function execOf(program: string, args: string): string {
  const quoted = quoteProgram(program.trim());
  return args.trim() ? `${quoted} ${args.trim()}` : quoted;
}

/** The menu's main categories, the ones an entry should name one of. */
export const MAIN_CATEGORIES = [
  'AudioVideo',
  'Development',
  'Education',
  'Game',
  'Graphics',
  'Network',
  'Office',
  'Science',
  'Settings',
  'System',
  'Utility',
] as const;

/** A blank entry, for starting without a location. */
export const BLANK_ENTRY = '[Desktop Entry]\nType=Application\nName=\nExec=\nTerminal=false\nCategories=Utility;\nX-Forager-Entry-Made=1\n';
