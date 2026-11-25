import { describe, expect, it } from 'vitest';
import { argsOf, execOf, parseFields, programOf, setKeys, withFields } from './entryText';

const ENTRY =
  '[Desktop Entry]\nType=Application\nName=Tool\nName[de]=Werkzeug\nExec=tool %U\n# a comment\nX-Other=1\n\n[Desktop Action new]\nName=New window\nExec=tool --new\n';

describe('editing a menu entry by its fields', () => {
  it('changes only the lines of the fields, as forager-core does', () => {
    const f = parseFields(ENTRY);
    expect(f.name).toBe('Tool');
    expect(f.exec).toBe('tool %U');
    const text = withFields(ENTRY, { ...f, name: 'Better tool', comment: 'Does things', noDisplay: true });
    expect(text).toBe(
      '[Desktop Entry]\nType=Application\nName=Better tool\nName[de]=Werkzeug\nExec=tool %U\n# a comment\nX-Other=1\nComment=Does things\nTerminal=false\nNoDisplay=true\n\n[Desktop Action new]\nName=New window\nExec=tool --new\n',
    );
    expect(withFields(text, parseFields(text))).toBe(text);
  });

  it('removes a field emptied in the form', () => {
    const text = withFields(ENTRY, { ...parseFields(ENTRY), comment: 'x' });
    const cleared = withFields(text, { ...parseFields(text), comment: '' });
    expect(cleared).not.toContain('Comment=');
  });

  it('starts an entry that has no main group', () => {
    expect(setKeys('', [['Name', 'Tool'], ['Comment', null]])).toBe('[Desktop Entry]\nType=Application\nName=Tool\n');
  });

  it('splits and builds the program and its arguments', () => {
    const exec = execOf('/opt/My App/$bin/run', '-desktop %F');
    expect(exec).toBe('"/opt/My App/\\$bin/run" -desktop %F');
    expect(programOf(exec)).toBe('/opt/My App/$bin/run');
    expect(argsOf(exec)).toBe('-desktop %F');
    expect(execOf('/usr/local/bin/tool', '')).toBe('/usr/local/bin/tool');
    expect(programOf('tool --x')).toBe('tool');
  });
});
