import { writeTextFile } from '@tauri-apps/api/fs';
import { save } from '@tauri-apps/api/dialog';

import { buildMarkdown, parseDetail } from './wordbook_format.js';
import { getDB } from './wordbook.js';

// 只有点按钮才会走到这里，绝不后台自动写文件（PRD §3.1）。
// dialog.save 选中的路径会被 tauri 运行时加进 fs scope，所以静态 scope 只写了
// $APPCONFIG 也照样能写到桌面（tauri-1.8.1 endpoints/dialog.rs:259）。
export async function exportMarkdown() {
    const db = await getDB();
    const rows = await db.select('SELECT * FROM entries WHERE deleted=0');
    const path = await save({ defaultPath: '我的生词本.md', filters: [{ name: 'Markdown', extensions: ['md'] }] });
    if (!path) return null;
    await writeTextFile(path, buildMarkdown(rows.map((r) => ({ ...r, detail: parseDetail(r.detail) }))));
    return path;
}
