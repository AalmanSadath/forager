import { useEffect, useMemo, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import * as ipc from '../ipc';
import { useAppStore } from '../store/appStore';
import type { EntrySummary, EntryValidation, IconChoice } from '../types';
import {
  BLANK_ENTRY,
  MAIN_CATEGORIES,
  argsOf,
  execOf,
  parseFields,
  programOf,
  withFields,
  type EntryFields,
} from '../entryText';
import ConfirmModal from './shared/ConfirmModal';
import Dropdown from './shared/Dropdown';
import { BackIcon } from './shared/icons';
import { EntryIcon } from './DesktopEntriesPanel';

type Mode = 'fields' | 'text';

/**
 * Making or changing one menu entry: pick where the app is and Forager finds
 * its program and icon, then adjust the common fields, or the whole file as
 * text. The text is what is saved; the fields are a view of it.
 */
export default function DesktopEntryPage({ id }: { id?: string }) {
  const setPage = useAppStore((s) => s.setPage);
  const toast = useAppStore((s) => s.toast);
  const isNew = id === undefined;

  const [summary, setSummary] = useState<EntrySummary | null>(null);
  const [existing, setExisting] = useState<string[]>([]);
  const [original, setOriginal] = useState<string | null>(isNew ? BLANK_ENTRY : null);
  const [text, setText] = useState(isNew ? BLANK_ENTRY : '');
  const [entryId, setEntryId] = useState(id ?? '');
  const [mode, setMode] = useState<Mode>('fields');
  const [error, setError] = useState<string | null>(null);
  // Found from the app's location.
  const [programs, setPrograms] = useState<string[]>([]);
  const [icons, setIcons] = useState<IconChoice[]>([]);
  // An icon file to copy into the theme on saving, and how it looks.
  const [iconFile, setIconFile] = useState<string | null>(null);
  const [iconUrl, setIconUrl] = useState<string | null>(null);
  const [checked, setChecked] = useState<EntryValidation>({ errors: [], notes: [] });
  const [busy, setBusy] = useState(false);
  const [dialog, setDialog] = useState<'delete' | 'discard' | 'replace' | null>(null);

  useEffect(() => {
    let live = true;
    ipc
      .listEntries()
      .then((list) => {
        if (!live) return;
        setExisting(list.map((e) => e.id));
        const mine = list.find((e) => e.id === id) ?? null;
        setSummary(mine);
        setIconUrl(mine?.icon_url ?? null);
      })
      .catch(() => {});
    if (id !== undefined) {
      ipc
        .readEntry(id)
        .then((read) => {
          if (!live) return;
          setOriginal(read);
          setText(read);
        })
        .catch((e) => live && setError(String(e)));
    }
    return () => {
      live = false;
    };
  }, [id]);

  // Checked as it is typed, a moment after the typing stops.
  useEffect(() => {
    if (!text) return;
    const timer = setTimeout(() => {
      ipc
        .validateEntry(text)
        .then(setChecked)
        .catch(() => {});
    }, 400);
    return () => clearTimeout(timer);
  }, [text]);

  const fields = useMemo(() => parseFields(text), [text]);
  const readOnly = !!summary?.owner;
  const dirty = (original !== null && text !== original) || iconFile !== null;
  const program = programOf(fields.exec);
  const args = argsOf(fields.exec);
  const setFields = (change: Partial<EntryFields>) => setText((t) => withFields(t, { ...parseFields(t), ...change }));

  const back = () => (dirty && !readOnly ? setDialog('discard') : setPage({ kind: 'entries' }));
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !dialog) back();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  const chooseIcon = async (path: string, url?: string) => {
    setIconFile(path);
    setIconUrl(url ?? (await ipc.iconPreview(path).catch(() => null)));
  };

  // Where the app is: a program, or the folder it is installed in.
  const locate = async (directory: boolean) => {
    const path = await open({ multiple: false, directory, title: directory ? 'Folder the app is installed in' : 'Program to start' });
    if (typeof path !== 'string') return;
    setError(null);
    setBusy(true);
    try {
      const found = await ipc.detectEntry(path);
      const { detected } = found;
      if (!detected.program) {
        setError(`Found no program in ${path}. Choose the program itself instead.`);
        return;
      }
      setPrograms(detected.programs);
      setIcons(detected.icons);
      const exec = await ipc.execLine(detected.program, args || detected.args);
      setText((t) => withFields(t, { ...parseFields(t), name: fields.name || detected.name, exec }));
      if (isNew && !entryId) setEntryId(detected.id);
      if (detected.icons[0]) await chooseIcon(detected.icons[0].path, detected.icons[0].data_url);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const browseProgram = async () => {
    const path = await open({ multiple: false, directory: false, title: 'Program to start' });
    if (typeof path === 'string') setFields({ exec: await ipc.execLine(path, args) });
  };

  const browseFolder = async () => {
    const path = await open({ multiple: false, directory: true, title: 'Folder the app starts in' });
    if (typeof path === 'string') setFields({ path });
  };

  const browseIcon = async () => {
    const path = await open({
      multiple: false,
      directory: false,
      title: 'Icon',
      filters: [{ name: 'Icons', extensions: ['png', 'svg', 'PNG', 'SVG'] }],
    });
    if (typeof path === 'string') await chooseIcon(path);
  };

  const save = async (replace = false) => {
    const saveId = entryId.trim();
    if (isNew && existing.includes(saveId) && !replace) {
      setDialog('replace');
      return;
    }
    setDialog(null);
    setBusy(true);
    setError(null);
    try {
      const saved = await ipc.saveEntry(saveId, text, iconFile);
      toast('success', `Saved ${saved.name} to the menu.`);
      if (isNew) {
        setPage({ kind: 'entry', id: saved.id });
        return;
      }
      setSummary(saved);
      setIconUrl(saved.icon_url);
      setIconFile(null);
      const read = await ipc.readEntry(saveId);
      setOriginal(read);
      setText(read);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    setDialog(null);
    try {
      await ipc.removeEntry(id!);
      toast('success', `Deleted ${summary?.name ?? id}.`);
      setPage({ kind: 'entries' });
    } catch (e) {
      setError(String(e));
    }
  };

  const tryIt = async () => {
    try {
      await ipc.launchEntry(id!);
    } catch (e) {
      toast('error', String(e));
    }
  };

  const idValid = /^[A-Za-z0-9_-][A-Za-z0-9._-]*$/.test(entryId.trim());
  const canSave = !readOnly && !busy && dirty && idValid && checked.errors.length === 0;
  const category = fields.categories.split(';').filter(Boolean);
  const categoryValue = category.length === 1 && (MAIN_CATEGORIES as readonly string[]).includes(category[0]) ? category[0] : '';

  if (original === null && !error) return <div className="panel"><p className="list-empty">Reading the entry…</p></div>;

  return (
    <div className="panel app-page">
      <header className="page-header">
        <button className="icon-btn" onClick={back} title="Back to desktop entries (Esc)">
          <BackIcon />
        </button>
        <div className="page-header-icon">
          <EntryIcon url={iconUrl} name={fields.name || entryId || '?'} size={26} />
        </div>
        <div className="page-header-title">
          <h1>{fields.name || (isNew ? 'New desktop entry' : id)}</h1>
          <span className="page-header-sub mono">{entryId ? `${entryId}.desktop` : 'not saved yet'}</span>
        </div>
        <div className="page-header-actions">
          {!isNew && !readOnly && (
            <button className="btn-secondary" onClick={tryIt} disabled={dirty} title={dirty ? 'Save first' : 'Start it as the menu would'}>
              Try it
            </button>
          )}
          {!readOnly && (
            <button className="btn-primary" onClick={() => save()} disabled={!canSave}>
              {busy ? 'Working…' : 'Save'}
            </button>
          )}
        </div>
      </header>

      {readOnly && (
        <p className="form-hint entries-intro">
          Forager writes this entry for {summary?.owner} and rewrites it on each update, so it is shown here read-only.{' '}
          <button className="link-btn" onClick={() => setPage({ kind: 'app', id: summary!.owner! })}>
            Open {summary?.owner}&rsquo;s page
          </button>
        </p>
      )}
      {error && <p className="form-error">{error}</p>}

      <div className="page-columns">
        <div className="page-column">
          {isNew && (
            <section className="page-card">
              <h3>Where the app is</h3>
              <p className="form-hint">
                Choose the program, or the folder the app is installed in (such as{' '}
                <span className="mono">/usr/local/MATLAB/R2024b</span>). Forager finds the program and icons to choose from.
              </p>
              <div className="entry-buttons">
                <button className="btn-secondary" onClick={() => locate(false)} disabled={busy}>Choose program…</button>
                <button className="btn-secondary" onClick={() => locate(true)} disabled={busy}>Choose folder…</button>
              </div>
              {programs.length > 1 && (
                <div className="form-group entry-programs">
                  <label>Program</label>
                  <Dropdown<string>
                    value={program}
                    ariaLabel="Program found in the folder"
                    onChange={async (p) => setFields({ exec: await ipc.execLine(p, args) })}
                    options={[
                      ...(programs.includes(program) ? [] : [{ value: program, label: program || 'None' }]),
                      ...programs.map((p) => ({ value: p, label: p })),
                    ]}
                  />
                </div>
              )}
            </section>
          )}

          <section className="page-card">
            <div className="page-card-header">
              <h3>Entry</h3>
              <div className="toggle-row">
                {(['fields', 'text'] as Mode[]).map((m) => (
                  <button key={m} className={`toggle-btn${mode === m ? ' active' : ''}`} onClick={() => setMode(m)}>
                    {m === 'fields' ? 'Fields' : 'Text'}
                  </button>
                ))}
              </div>
            </div>
            {mode === 'text' ? (
              <textarea
                className="mono entry-text"
                aria-label="Entry text"
                value={text}
                readOnly={readOnly}
                spellCheck={false}
                onChange={(e) => setText(e.target.value)}
              />
            ) : (
              <fieldset className="entry-fields" disabled={readOnly}>
                {isNew && (
                  <div className="form-group">
                    <label htmlFor="entry-id">File name</label>
                    <input id="entry-id" className="mono" value={entryId} onChange={(e) => setEntryId(e.target.value)} />
                    <p className="form-hint">
                      Saved as <span className="mono">{entryId || '…'}.desktop</span>. Letters, digits, &lsquo;.&rsquo;, &lsquo;_&rsquo;
                      and &lsquo;-&rsquo;.
                    </p>
                  </div>
                )}
                <div className="form-group">
                  <label htmlFor="entry-name">Name</label>
                  <input id="entry-name" value={fields.name} onChange={(e) => setFields({ name: e.target.value })} />
                </div>
                <div className="form-group">
                  <label htmlFor="entry-program">Program</label>
                  <div className="lookup-row">
                    <input
                      id="entry-program"
                      className="mono"
                      value={program}
                      spellCheck={false}
                      onChange={(e) => setFields({ exec: execOf(e.target.value, args) })}
                    />
                    <button className="btn-secondary" onClick={browseProgram}>Browse…</button>
                  </div>
                </div>
                <div className="form-group">
                  <label htmlFor="entry-args">Arguments</label>
                  <input
                    id="entry-args"
                    className="mono"
                    value={args}
                    placeholder="e.g. -desktop %F"
                    spellCheck={false}
                    onChange={(e) => setFields({ exec: execOf(program, e.target.value) })}
                  />
                  <p className="form-hint">%F passes files opened with the app, %U links.</p>
                </div>
                <div className="form-group">
                  <label htmlFor="entry-comment">Description</label>
                  <input id="entry-comment" value={fields.comment} onChange={(e) => setFields({ comment: e.target.value })} />
                </div>
                <div className="form-group">
                  <label>Category</label>
                  <Dropdown<string>
                    value={categoryValue}
                    ariaLabel="Menu category"
                    onChange={(c) => c && setFields({ categories: `${c};` })}
                    options={[
                      ...(categoryValue ? [] : [{ value: '', label: fields.categories ? `Custom (${fields.categories})` : 'None' }]),
                      ...MAIN_CATEGORIES.map((c) => ({ value: c, label: c })),
                    ]}
                  />
                </div>
                <label className="checkbox-row">
                  <input type="checkbox" checked={fields.terminal} onChange={(e) => setFields({ terminal: e.target.checked })} />
                  <span>Run in a terminal</span>
                </label>
                <p className="form-hint entry-check-hint">
                  Only for programs that work in a terminal: the terminal stays open while they run. An app that opens no
                  window without one usually needs an argument instead, such as MATLAB&rsquo;s <span className="mono">-desktop</span>.
                </p>
                <label className="checkbox-row">
                  <input type="checkbox" checked={fields.noDisplay} onChange={(e) => setFields({ noDisplay: e.target.checked })} />
                  <span>Hide from the menu</span>
                </label>
                <details className="entry-more">
                  <summary>More</summary>
                  <div className="form-group">
                    <label htmlFor="entry-wmclass">Window class</label>
                    <input id="entry-wmclass" className="mono" value={fields.wmClass} onChange={(e) => setFields({ wmClass: e.target.value })} />
                    <p className="form-hint">
                      The class or app id the app&rsquo;s windows have. Set it when the running app shows a generic icon in the dock.
                    </p>
                  </div>
                  <div className="form-group">
                    <label htmlFor="entry-path">Start in folder</label>
                    <div className="lookup-row">
                      <input
                        id="entry-path"
                        className="mono"
                        value={fields.path}
                        placeholder="Your home folder"
                        spellCheck={false}
                        onChange={(e) => setFields({ path: e.target.value })}
                      />
                      <button className="btn-secondary" onClick={browseFolder}>Browse…</button>
                    </div>
                    <p className="form-hint">The folder the app starts in, such as where it keeps its files.</p>
                  </div>
                  <div className="form-group">
                    <label htmlFor="entry-icon-name">Icon name or path</label>
                    <input id="entry-icon-name" className="mono" value={fields.icon} onChange={(e) => setFields({ icon: e.target.value })} />
                  </div>
                </details>
              </fieldset>
            )}
            {checked.errors.map((e) => (
              <p key={e} className="form-hint form-hint-error">{e}</p>
            ))}
            {checked.notes.map((n) => (
              <p key={n} className="form-hint form-hint-warn">{n}</p>
            ))}
          </section>
        </div>

        <div className="page-column">
          <section className="page-card">
            <h3>Icon</h3>
            <div className="entry-icon-current">
              <EntryIcon url={iconUrl} name={fields.name || '?'} size={64} />
              <div>
                <p className="form-hint">
                  {iconFile ? (
                    <>
                      <span className="mono">{iconFile}</span> is copied into your icon theme when you save, so the entry keeps
                      it if the app moves.
                    </>
                  ) : fields.icon ? (
                    <>
                      Uses <span className="mono">{fields.icon}</span>.
                    </>
                  ) : (
                    'No icon yet.'
                  )}
                </p>
                {!readOnly && (
                  <button className="btn-secondary btn-sm" onClick={browseIcon}>Choose file…</button>
                )}
              </div>
            </div>
            {icons.length > 0 && !readOnly && (
              <>
                <p className="form-hint">Found near the program:</p>
                <div className="entry-icon-choices">
                  {icons.map((icon) => (
                    <button
                      key={icon.path}
                      className={`entry-icon-choice${iconFile === icon.path ? ' active' : ''}`}
                      title={icon.path}
                      aria-label={`Use ${icon.path}`}
                      onClick={() => chooseIcon(icon.path, icon.data_url)}
                    >
                      <img src={icon.data_url} width={40} height={40} alt="" />
                    </button>
                  ))}
                </div>
              </>
            )}
          </section>

          {!isNew && !readOnly && (
            <section className="page-card">
              <h3>Remove</h3>
              <p className="form-hint">Deletes the entry from the menu, and the icon Forager copied for it. The app stays.</p>
              <button className="btn-danger" onClick={() => setDialog('delete')}>Delete entry</button>
            </section>
          )}
        </div>
      </div>

      {dialog === 'delete' && (
        <ConfirmModal
          question={`Delete ${fields.name || id}?`}
          hint={
            <>
              Removes <span className="mono">{summary?.file ?? `${id}.desktop`}</span> and its copied icon. The app itself stays.
            </>
          }
          confirmLabel="Delete"
          onConfirm={remove}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog === 'discard' && (
        <ConfirmModal
          question="Discard your changes?"
          hint="They have not been saved."
          confirmLabel="Discard"
          onConfirm={() => setPage({ kind: 'entries' })}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog === 'replace' && (
        <ConfirmModal
          question={`Replace ${entryId.trim()}.desktop?`}
          hint="An entry of that name is already in your menu."
          confirmLabel="Replace"
          onConfirm={() => save(true)}
          onCancel={() => setDialog(null)}
        />
      )}
    </div>
  );
}
