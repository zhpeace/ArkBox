<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open, save } from "@tauri-apps/plugin-dialog";
import { open as shellOpen } from "@tauri-apps/plugin-shell";

interface EntryInfo {
  name: string;
  size: number;
  compressed_size: number | null;
  is_dir: boolean;
  modified: number | null;
  method: string | null;
}
interface ArchiveInfo {
  path: string;
  format: string;
  entry_count: number;
  total_size: number;
}
interface PreviewData {
  name: string;
  mime: string;
  text: string | null;
  data_base64: string | null;
  truncated: boolean;
  is_binary: boolean;
}
interface TestResult {
  ok: boolean;
  entries: { name: string; ok: boolean; error: string | null }[];
}
interface PresetConfig {
  name: string;
  format: string;
  password: string | null;
  level: number;
  exclude: string[];
  compress_hidden: boolean;
}
interface EditHandle {
  handle_id: string;
  temp_path: string;
  entry_name: string;
}
interface ProgressData {
  kind: string;
  current: number;
  total: number;
}

const tab = ref<"compress" | "browse">("compress");
const logs = ref<string[]>([]);
const fmtBytes = (n: number) =>
  n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(2)} MB`;

function log(msg: string) {
  logs.value.push(`[${new Date().toLocaleTimeString()}] ${msg}`);
}
function logErr(msg: string) {
  logs.value.push(`[${new Date().toLocaleTimeString()}] ✗ ${msg}`);
}

/* ---------- 压缩 ---------- */
const compressPaths = ref<string[]>([]);
const destPath = ref("");
const format = ref("zip");
const password = ref("");
const level = ref(6);
const excludeText = ref(".DS_Store\nnode_modules/\n.git/");
const compressHidden = ref(false);
const presets = ref<PresetConfig[]>([]);
const presetName = ref("");
const progress = ref<ProgressData | null>(null);
const verified = ref<TestResult | null>(null);
const pct = computed(() =>
  progress.value && progress.value.total > 0
    ? Math.min(100, Math.round((progress.value.current / progress.value.total) * 100))
    : 0
);

async function pickCompressPaths() {
  const r = await open({ multiple: true, directory: false });
  if (r) compressPaths.value = Array.isArray(r) ? r : [r as string];
}
async function pickDest() {
  const r = await save({ defaultPath: `archive.${extFor(format.value)}` });
  if (r) destPath.value = r as string;
}
function extFor(f: string) {
  switch (f) {
    case "zip":
      return "zip";
    case "7z":
      return "7z";
    case "targz":
      return "tar.gz";
    case "tarbz":
      return "tar.bz2";
    case "tarxz":
      return "tar.xz";
    case "tarzstd":
      return "tar.zst";
    case "gz":
      return "gz";
    case "bz2":
      return "bz2";
    case "xz":
      return "xz";
    case "zst":
      return "zst";
    default:
      return "zip";
  }
}
async function doCompress() {
  if (compressPaths.value.length === 0) return log("请先选择要压缩的文件/目录");
  if (!destPath.value) return log("请选择输出路径");
  verified.value = null;
  const exclude = excludeText.value
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter(Boolean);
  try {
    const info: ArchiveInfo = await invoke("compress", {
      paths: compressPaths.value,
      dest: destPath.value,
      format: format.value,
      password: password.value || null,
      level: level.value,
      exclude,
      compressHidden: compressHidden.value,
    });
    log(`已压缩: ${info.path} (${info.entry_count} 项, ${fmtBytes(info.total_size)})`);
  } catch (e) {
    logErr(String(e));
  }
  progress.value = null;
}
async function loadPresets() {
  try {
    presets.value = await invoke("load_presets");
  } catch (e) {
    logErr(String(e));
  }
}
function applyPreset(name: string) {
  const p = presets.value.find((x) => x.name === name);
  if (!p) return;
  format.value = p.format;
  password.value = p.password || "";
  level.value = p.level;
  excludeText.value = p.exclude.join("\n");
  compressHidden.value = p.compress_hidden;
  presetName.value = p.name;
  log(`已套用预设: ${name}`);
}
async function deletePreset() {
  const name = presetName.value.trim();
  if (!name) return log("请输入要删除的预设名称");
  if (!presets.value.some((x) => x.name === name)) return log("没有这个预设");
  try {
    await invoke("delete_preset", { name });
    await loadPresets();
    presetName.value = "";
    log(`已删除预设: ${name}`);
  } catch (e) {
    logErr(String(e));
  }
}
async function savePreset() {
  if (!presetName.value.trim()) return log("请输入预设名称");
  const cfg: PresetConfig = {
    name: presetName.value.trim(),
    format: format.value,
    password: password.value || null,
    level: level.value,
    exclude: excludeText.value.split(/\r?\n/).map((s) => s.trim()).filter(Boolean),
    compress_hidden: compressHidden.value,
  };
  try {
    await invoke("save_preset", { cfg });
    await loadPresets();
    log(`已保存预设: ${cfg.name}`);
  } catch (e) {
    logErr(String(e));
  }
}

/* ---------- 浏览 / 解压 ---------- */
const archivePath = ref("");
const entries = ref<EntryInfo[]>([]);
const selected = ref<Record<string, boolean>>({});
const preview = ref<PreviewData | null>(null);
const browsePassword = ref("");
const extractDest = ref("");
const testResult = ref<TestResult | null>(null);
const editing = ref<EditHandle | null>(null);

async function openArchive() {
  const r = await open({ multiple: false, directory: false });
  if (r) {
    archivePath.value = r as string;
    await refreshList();
  }
}

/* ---------- 系统集成：文件关联 / 拖放 ---------- */
const ARCHIVE_EXT = ["zip", "7z", "tar", "gz", "bz2", "xz", "zst", "tgz", "tbz", "txz", "rar"];
function isArchivePath(p: string): boolean {
  const lower = p.toLowerCase();
  if (ARCHIVE_EXT.some((e) => lower.endsWith(`.${e}`))) return true;
  // RAR 分卷命名：xxx.r00 / xxx.001
  return /\.(r\d{2}|\d{3})$/.test(lower);
}

async function openAsArchive(p: string) {
  tab.value = "browse";
  archivePath.value = p;
  await refreshList();
}

function stageCompress(paths: string[]) {
  tab.value = "compress";
  compressPaths.value = paths;
}

function handleDrop(paths: string[]) {
  if (paths.length === 1 && isArchivePath(paths[0])) {
    // 拖入一个压缩包 -> 直接浏览
    void openAsArchive(paths[0]);
  } else {
    // 多个文件/目录/非压缩包 -> 预填压缩列表
    stageCompress(paths);
  }
}
async function refreshList() {
  if (!archivePath.value) return;
  entries.value = [];
  selected.value = {};
  testResult.value = null;
  preview.value = null;
  try {
    entries.value = await invoke("list_entries", {
      archive: archivePath.value,
      password: browsePassword.value || null,
    });
    log(`已读取: ${entries.value.length} 项`);
  } catch (e) {
    logErr(String(e));
  }
}
function isSel(name: string) {
  return !!selected.value[name];
}
function toggleSel(name: string) {
  selected.value[name] = !selected.value[name];
}
async function previewRow(entry: EntryInfo) {
  if (entry.is_dir) return;
  try {
    preview.value = await invoke("preview_entry", {
      archive: archivePath.value,
      entry: entry.name,
      password: browsePassword.value || null,
      maxBytes: 200000,
    });
  } catch (e) {
    logErr(String(e));
  }
}
function selectedNames(): string[] {
  return entries.value.filter((e) => selected.value[e.name]).map((e) => e.name);
}
async function extractSelected() {
  const names = selectedNames();
  if (names.length === 0) return log("请勾选要解压的条目");
  const r = await open({ multiple: false, directory: true });
  if (!r) return;
  extractDest.value = r as string;
  try {
    const res = await invoke("extract", {
      archive: archivePath.value,
      dest: extractDest.value,
      entries: names,
      password: browsePassword.value || null,
    });
    log(`已解压 ${names.length} 项 -> ${extractDest.value}`);
    void res;
  } catch (e) {
    logErr(String(e));
  }
  progress.value = null;
}
async function extractAll() {
  const r = await open({ multiple: false, directory: true });
  if (!r) return;
  try {
    const res = await invoke("extract", {
      archive: archivePath.value,
      dest: r,
      entries: null,
      password: browsePassword.value || null,
    });
    log(`已全量解压 -> ${r as string}`);
    void res;
  } catch (e) {
    logErr(String(e));
  }
  progress.value = null;
}
async function testArchive() {
  try {
    testResult.value = await invoke("test_archive", {
      archive: archivePath.value,
      password: browsePassword.value || null,
    });
    log(testResult.value?.ok ? "完整性检测通过 ✓" : "检测到损坏条目 ✗");
  } catch (e) {
    logErr(String(e));
  }
}
async function beginEdit(entryName: string) {
  try {
    const h: EditHandle = await invoke("begin_edit_entry", {
      archive: archivePath.value,
      entry: entryName,
      password: browsePassword.value || null,
    });
    editing.value = h;
    await shellOpen(h.temp_path);
    log(`已打开编辑: ${entryName}（改完点“完成编辑”）`);
  } catch (e) {
    logErr(String(e));
  }
}
async function commitEdit() {
  if (!editing.value) return;
  try {
    await invoke("commit_edit", {
      handle: editing.value,
      archive: archivePath.value,
      password: browsePassword.value || null,
    });
    log(`已写回: ${editing.value.entry_name}`);
    editing.value = null;
    await refreshList();
  } catch (e) {
    logErr(String(e));
  }
}
async function cancelEdit() {
  if (!editing.value) return;
  try {
    await invoke("cancel_edit", { handle: editing.value });
  } catch (e) {
    logErr(String(e));
  }
  editing.value = null;
}

async function onReady() {
  await loadPresets();
  // 冷启动：文件关联 / 右键服务传入的路径（单压缩包->浏览，多文件/目录->压缩）
  try {
    const pending = (await invoke("take_pending_open")) as string[];
    if (pending.length) handleDrop(pending);
  } catch (e) {
    logErr(String(e));
  }
  // 运行中：再次双击另一个压缩包 / 从右键服务再次传入
  listen("opened-files", (e) => {
    const paths = e.payload as string[];
    if (paths.length) handleDrop(paths);
  }).catch((e) => logErr(String(e)));
  // 压缩 / 解压进度
  listen("archive-progress", (e) => {
    progress.value = e.payload as ProgressData;
  }).catch((e) => logErr(String(e)));
  // 压缩后自动校验结果
  listen("archive-verified", (e) => {
    const t = e.payload as TestResult;
    verified.value = t;
    log(t.ok ? "压缩后自动校验通过 ✓" : "压缩后自动校验发现问题 ✗");
  }).catch((e) => logErr(String(e)));
  // 拖放：一个压缩包->浏览，其余->预填压缩列表
  void getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type === "drop") handleDrop(event.payload.paths);
  });
}
onMounted(onReady);
</script>

<template>
  <div style="display: flex; flex-direction: column; height: 100%">
    <div class="tabs">
      <div :class="['tab', tab === 'compress' && 'active']" @click="tab = 'compress'">
        压缩
      </div>
      <div :class="['tab', tab === 'browse' && 'active']" @click="tab = 'browse'">
        浏览 / 解压
      </div>
    </div>

    <div v-if="progress" class="panel" style="margin: 0 14px; padding: 10px 14px">
      <div class="row" style="gap: 10px">
        <span class="muted">{{ progress.kind === "compress" ? "压缩中" : "解压中" }}</span>
        <div class="bar">
          <div
            class="bar-fill"
            :class="{ indeterminate: progress.total === 0 }"
            :style="progress.total > 0 ? { width: pct + '%' } : {}"
          ></div>
        </div>
        <span class="muted" v-if="progress.total > 0">{{ pct }}%</span>
        <span class="muted" v-else>处理中…</span>
      </div>
    </div>

    <div style="padding: 14px; overflow: auto; flex: 1">
      <!-- 压缩 -->
      <div v-if="tab === 'compress'" class="col" style="gap: 14px">
        <div class="panel col">
          <div class="row">
            <button @click="pickCompressPaths">选择文件/目录</button>
            <span class="muted">{{ compressPaths.length }} 项已选</span>
          </div>
          <div class="list" v-if="compressPaths.length">
            <div class="item" v-for="p in compressPaths" :key="p">
              <span class="name">{{ p }}</span>
            </div>
          </div>
          <div class="row">
            <span class="label">格式</span>
            <select v-model="format">
              <option value="zip">ZIP (支持加密)</option>
              <option value="7z">7Z (支持加密)</option>
              <option value="targz">TAR.GZ</option>
              <option value="tarbz">TAR.BZ2</option>
            <option value="tarxz">TAR.XZ</option>
            <option value="tarzstd">TAR.ZST</option>
            <option value="gz">GZ (单文件)</option>
            <option value="bz2">BZ2 (单文件)</option>
            <option value="xz">XZ (单文件)</option>
            <option value="zst">ZST (单文件)</option>
            </select>
            <span class="label">等级</span>
            <input type="number" min="0" max="9" v-model.number="level" style="width: 64px" />
            <label class="row" style="gap: 4px">
              <input type="checkbox" v-model="compressHidden" /> 包含隐藏文件
            </label>
          </div>
          <div class="row">
            <span class="label">密码</span>
            <input class="grow" type="password" v-model="password" placeholder="留空不加密" />
          </div>
          <div class="row">
            <span class="label">排除</span>
            <textarea
              v-model="excludeText"
              rows="3"
              style="flex: 1; background: var(--panel); color: var(--text); border: 1px solid var(--border); border-radius: 6px; padding: 6px; font-size: 12px"
            ></textarea>
          </div>
          <div class="row">
            <button class="primary" @click="doCompress">压缩</button>
            <input v-model="destPath" placeholder="输出路径" style="flex: 1" />
            <button @click="pickDest">选择</button>
          </div>
          <div class="row" v-if="verified" style="margin-top: 4px">
            <span :class="['pill', verified.ok ? 'ok' : 'err']">
              {{ verified.ok ? "压缩后校验通过 ✓" : "压缩后校验未通过 ✗" }}
            </span>
          </div>
        </div>

        <div class="panel col">
          <div class="row">
            <span class="label">预设</span>
            <select @change="applyPreset(($event.target as HTMLSelectElement).value)">
              <option value="">— 套用预设 —</option>
              <option v-for="p in presets" :key="p.name" :value="p.name">
                {{ p.name }} ({{ p.format }})
              </option>
            </select>
            <input v-model="presetName" placeholder="预设名称" />
            <button @click="savePreset">保存当前为预设</button>
            <button @click="deletePreset">删除预设</button>
          </div>
        </div>
      </div>

      <!-- 浏览 -->
      <div v-else class="col" style="gap: 14px">
        <div class="panel col">
          <div class="row">
            <button @click="openArchive">打开压缩包</button>
            <input class="grow" v-model="archivePath" placeholder="压缩包路径" />
            <input
              type="password"
              v-model="browsePassword"
              placeholder="密码(如需)"
              style="width: 140px"
            />
            <button @click="refreshList">刷新</button>
          </div>
          <div class="row">
            <button @click="extractAll">全量解压</button>
            <button @click="extractSelected">解压选中</button>
            <button @click="testArchive">完整性检测</button>
            <span class="muted" v-if="editing">编辑中: {{ editing.entry_name }}</span>
            <button v-if="editing" class="primary" @click="commitEdit">完成编辑</button>
            <button v-if="editing" @click="cancelEdit">取消编辑</button>
          </div>
        </div>

        <div class="row" style="align-items: stretch; gap: 14px">
          <div class="panel grow col">
            <div class="muted">条目（勾选=解压/选中，点击=预览）</div>
            <div class="list">
              <div
                v-for="e in entries"
                :key="e.name"
                :class="['item', preview && preview.name === e.name && 'sel']"
                @click="previewRow(e)"
              >
                <input
                  type="checkbox"
                  :checked="isSel(e.name)"
                  @click.stop="toggleSel(e.name)"
                />
                <span class="name">{{ e.name }}{{ e.is_dir ? "/" : "" }}</span>
                <span class="meta" v-if="!e.is_dir">{{ fmtBytes(e.size) }}</span>
              </div>
              <div v-if="!entries.length" class="muted" style="padding: 10px">
                尚未打开压缩包
              </div>
            </div>
          </div>

          <div class="panel grow col">
            <div class="muted">预览</div>
            <div v-if="preview" class="preview">
              <div v-if="preview.is_binary && preview.data_base64">
                <img
                  v-if="preview.mime.startsWith('image/')"
                  :src="`data:${preview.mime};base64,${preview.data_base64}`"
                />
                <span v-else class="muted">二进制文件，无法预览 ({{ preview.mime }})</span>
              </div>
              <pre v-else>{{ preview.text }}</pre>
              <div v-if="preview.truncated" class="muted">（已截断）</div>
            </div>
            <div v-else class="muted">点击左侧条目预览</div>
            <div v-if="preview" class="row" style="margin-top: 8px">
              <button @click="beginEdit(preview.name)">
                直接编辑此文件
              </button>
            </div>
          </div>
        </div>

        <div class="panel col" v-if="testResult">
          <div class="muted">完整性检测</div>
          <div class="row">
            <span :class="['pill', testResult?.ok ? 'ok' : 'err']">
              {{ testResult?.ok ? "全部通过" : "存在问题" }}
            </span>
          </div>
          <div
            v-for="t in (testResult?.entries || []).filter((x) => !x.ok)"
            :key="t.name"
            class="muted"
          >
            ✗ {{ t.name }} — {{ t.error }}
          </div>
        </div>
      </div>

      <div class="panel" style="margin-top: 14px">
        <div class="muted" style="margin-bottom: 6px">日志</div>
        <div class="log">{{ logs.join("\n") }}</div>
      </div>
    </div>
  </div>
</template>
