import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const WEB_ROOT = resolve(__dirname, '..');
const TOKENS_CSS = join(WEB_ROOT, 'src/styles/tokens.css');

function collectTsxFiles(dir: string): string[] {
  if (!existsSync(dir)) return [];
  const entries = readdirSync(dir, { withFileTypes: true });
  const files: string[] = [];
  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...collectTsxFiles(full));
    } else if (entry.name.endsWith('.tsx') || entry.name.endsWith('.ts')) {
      files.push(full);
    }
  }
  return files;
}

function extractCssVarNames(block: string): string[] {
  const matches = block.matchAll(/(--[a-z0-9-]+)\s*:/g);
  return Array.from(matches, (m) => m[1]).sort();
}

describe('静态设计守卫', () => {
  it('亮色与暗色主题令牌键值必须严格对称', () => {
    const css = readFileSync(TOKENS_CSS, 'utf8');
    const lightBlock = css.match(/:root,\s*\[data-theme='light'\]\s*\{([\s\S]*?)\n\}/)?.[1];
    const darkBlock = css.match(/\[data-theme='dark'\]\s*\{([\s\S]*?)\n\}/)?.[1];

    expect(lightBlock).toBeTruthy();
    expect(darkBlock).toBeTruthy();

    const lightKeys = extractCssVarNames(lightBlock ?? '');
    const darkKeys = extractCssVarNames(darkBlock ?? '');
    expect(lightKeys.length).toBeGreaterThan(20);
    expect(darkKeys).toEqual(lightKeys);
  });

  it('页面与业务组件禁止硬编码色值、内联 style 与绕过 ui/ 的原生表单控件', () => {
    const targets = [
      ...collectTsxFiles(join(WEB_ROOT, 'src/pages')),
      ...collectTsxFiles(join(WEB_ROOT, 'src/components')),
    ];
    expect(targets.length).toBeGreaterThan(0);

    for (const file of targets) {
      const content = readFileSync(file, 'utf8');
      expect(content, `${file} 不应包含硬编码十六进制色值`).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
      expect(content, `${file} 不应包含 rgb/rgba/hsl 硬编码色值`).not.toMatch(/\b(?:rgb|rgba|hsl|hsla)\(/);
      expect(content, `${file} 不应使用内联 style={{...}}`).not.toMatch(/\bstyle=\{\{/);
      expect(content, `${file} 不应绕过 ui/ 编写原生 <button>`).not.toMatch(/<button\b/);
      expect(content, `${file} 不应绕过 ui/ 编写原生 <input>`).not.toMatch(/<input\b/);
      expect(content, `${file} 不应绕过 ui/ 编写原生 <select>`).not.toMatch(/<select\b/);
    }
  });
});
