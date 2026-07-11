import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { CaptureMeta, TreeNode, SortBy, SortDirection } from "@/types";
import {
	openDirectory as invokeOpenDir,
	getDirectoryTree as invokeGetTree,
} from "@/types/tauri";
import { listen } from "@tauri-apps/api/event";

interface ScanProgress {
	percent: number;
	path: string;
	phase: string;
}

export const useBrowseStore = defineStore("browse", () => {
	// 核心数据
	const captures = ref<CaptureMeta[]>([]);
	const filteredIndices = ref<number[]>([]);
	const directoryTree = ref<TreeNode[]>([]);
	const currentPath = ref<string>("");

	// 选中状态
	const selectedIndices = ref<Set<number>>(new Set());
	const focusedIndex = ref<number | null>(null);

	// 排序/筛选
	const sortBy = ref<SortBy>("FileName");
	const sortDirection = ref<SortDirection>("Ascending");
	const searchText = ref("");

	// 扫描进度
	const scanProgress = ref<ScanProgress | null>(null);
	const isScanning = ref(false);

	// 预览
	const zoomLevel = ref(1.0);
	const fitToWindow = ref(true);

	// 计算属性
	const totalCount = computed(() => captures.value.length);
	const filteredCount = computed(() => filteredIndices.value.length);
	const selectedCount = computed(() => selectedIndices.value.size);

	const filteredCaptures = computed(() =>
		filteredIndices.value.map((i) => captures.value[i]),
	);

	const selectedCaptures = computed(() =>
		Array.from(selectedIndices.value).map((i) => captures.value[i]),
	);

	// 操作
	async function loadDirectoryTree() {
		try {
			const tree = await invokeGetTree();
			if (tree.length > 0) {
				directoryTree.value = tree;
				return;
			}
		} catch {
			console.warn("Not in Tauri environment, using mock directory tree");
		}
		// Fallback: 不在 Tauri 环境时显示 mock 目录
		directoryTree.value = [
			{
				path: "/home/user/Pictures",
				name: "Pictures",
				isFavorite: false,
				hasChildren: true,
				children: [],
			},
			{
				path: "/media",
				name: "media",
				isFavorite: false,
				hasChildren: true,
				children: [],
			},
			{
				path: "/mnt",
				name: "mnt",
				isFavorite: false,
				hasChildren: true,
				children: [],
			},
		] as TreeNode[];
	}

	async function openDirectory(path: string) {
		isScanning.value = true;
		scanProgress.value = { percent: 0, path, phase: "scanning" };

		const unlisten = await listen<ScanProgress>("scan-progress", (event) => {
			scanProgress.value = event.payload;
			if (event.payload.percent >= 100) {
				isScanning.value = false;
			}
		}).catch(() => null); // 非 Tauri 环境会失败

		try {
			const result = await invokeOpenDir(path, ["xmp"]);
			captures.value = result.captures;
			filteredIndices.value = result.captures.map((_, i) => i);
			currentPath.value = path;
		} catch {
			// 不在 Tauri 环境时，用 mock 数据
			const names = [
				"DSC_0001",
				"DSC_0002",
				"DSC_0003",
				"DSC_0004",
				"DSC_0005",
				"DSC_0006",
				"DSC_0007",
				"DSC_0008",
				"DSC_0009",
				"DSC_0010",
				"IMG_2024",
				"IMG_2025",
				"IMG_2026",
				"PANO_001",
				"PANO_002",
				"Screenshot_2025",
				"Selfie_01",
				"Sunset_01",
				"Vacation_01",
				"Wedding_01",
			];
			const formats = ["JPEG", "NEF", "CR2", "ARW", "DNG", "PNG", "HEIF"];
			captures.value = names.map((name, i) => ({
				index: i,
				baseName: name,
				primaryPath: `/mock/photos/${name}.jpg`,
				primaryFormat: formats[i % formats.length],
				stackCount: i % 3 === 0 ? 1 + (i % 3) : 0,
				fileSize: 1024 * 1024 * (1 + (i % 10)),
				dateTaken: null,
				hasXmp: i % 5 === 0,
				extensions: [formats[i % formats.length]],
			}));
			filteredIndices.value = captures.value.map((_, i) => i);
			currentPath.value = path;
		}

		if (unlisten && typeof unlisten === "function") {
			unlisten();
			unlisten();
		}
		isScanning.value = false;
		scanProgress.value = null;
		selectedIndices.value = new Set();
		focusedIndex.value = null;
		applyFilters();
	}

	function selectCapture(idx: number) {
		selectedIndices.value = new Set([idx]);
	}

	function toggleSelect(idx: number) {
		const s = new Set(selectedIndices.value);
		if (s.has(idx)) s.delete(idx);
		else s.add(idx);
		selectedIndices.value = s;
	}

	function selectRange(idx: number) {
		if (selectedIndices.value.size === 0) {
			selectedIndices.value = new Set([idx]);
			return;
		}
		const sorted = Array.from(selectedIndices.value).sort((a, b) => a - b);
		const last = sorted[sorted.length - 1];
		const start = Math.min(last, idx);
		const end = Math.max(last, idx);
		const range = new Set<number>();
		for (let i = start; i <= end; i++) range.add(i);
		selectedIndices.value = range;
	}

	function selectAll() {
		selectedIndices.value = new Set(filteredIndices.value);
	}

	function invertSelection() {
		const all = new Set(filteredIndices.value);
		const s = new Set(selectedIndices.value);
		selectedIndices.value = new Set(Array.from(all).filter((i) => !s.has(i)));
	}

	function clearSelection() {
		selectedIndices.value = new Set();
		focusedIndex.value = null;
	}

	function setSort(by: SortBy, dir: SortDirection) {
		sortBy.value = by;
		sortDirection.value = dir;
		applyFilters();
	}

	function setSearch(text: string) {
		searchText.value = text;
		applyFilters();
	}

	function applyFilters() {
		let indices = captures.value.map((_, i) => i);
		if (searchText.value) {
			const q = searchText.value.toLowerCase();
			indices = indices.filter((i) =>
				captures.value[i].baseName.toLowerCase().includes(q),
			);
		}
		indices.sort((a, b) => {
			const ca = captures.value[a];
			const cb = captures.value[b];
			let cmp = 0;
			switch (sortBy.value) {
				case "FileName":
					cmp = ca.baseName.localeCompare(cb.baseName);
					break;
				case "FileSize":
					cmp = (ca.fileSize ?? 0) - (cb.fileSize ?? 0);
					break;
				case "DateTaken":
					cmp = (ca.dateTaken ?? "").localeCompare(cb.dateTaken ?? "");
					break;
			}
			return sortDirection.value === "Ascending" ? cmp : -cmp;
		});
		filteredIndices.value = indices;
	}

	function focusNext() {
		if (filteredIndices.value.length === 0) return;
		const next = (focusedIndex.value ?? -1) + 1;
		focusedIndex.value = Math.min(next, filteredIndices.value.length - 1);
	}

	function focusPrev() {
		if (filteredIndices.value.length === 0) return;
		const prev = (focusedIndex.value ?? filteredIndices.value.length) - 1;
		focusedIndex.value = Math.max(prev, 0);
	}

	function setZoom(delta: number) {
		zoomLevel.value = Math.max(0.25, Math.min(5.0, zoomLevel.value + delta));
		if (Math.abs(zoomLevel.value - 1.0) < 0.01) fitToWindow.value = true;
		else fitToWindow.value = false;
	}

	function toggleFitToWindow() {
		fitToWindow.value = !fitToWindow.value;
		if (fitToWindow.value) zoomLevel.value = 1.0;
	}

	return {
		captures,
		filteredIndices,
		directoryTree,
		currentPath,
		selectedIndices,
		focusedIndex,
		sortBy,
		sortDirection,
		searchText,
		scanProgress,
		isScanning,
		zoomLevel,
		fitToWindow,
		totalCount,
		filteredCount,
		selectedCount,
		filteredCaptures,
		selectedCaptures,
		openDirectory,
		selectCapture,
		toggleSelect,
		selectRange,
		selectAll,
		invertSelection,
		clearSelection,
		setSort,
		setSearch,
		applyFilters,
		focusNext,
		focusPrev,
		setZoom,
		toggleFitToWindow,
		loadDirectoryTree,
	};
});
