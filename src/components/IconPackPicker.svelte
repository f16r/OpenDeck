<script lang="ts">
	import Trash from "phosphor-svelte/lib/Trash";

	import { t } from "$lib/i18n";
	import { getWebserverUrl } from "$lib/ports";
	import { resizeImage } from "$lib/rendererHelper";

	import { invoke } from "@tauri-apps/api/core";
	import { ask, message, open } from "@tauri-apps/plugin-dialog";
	import { onMount } from "svelte";

	type Icon = { path: string; name: string; tags: string[] };
	type IconPack = { id: string; name: string; author: string; version: string; description: string; icon: string | null; icons: Icon[] };

	/** Called with the chosen icon as a data URL, so that the key keeps it when the pack is removed. */
	export let onPick: (image: string) => void;
	export let onClose: () => void;

	let packs: IconPack[] = [];
	let selected: string | undefined;
	let search = "";

	async function load() {
		packs = await invoke("list_icon_packs");
		if (!packs.some((p) => p.id == selected)) selected = packs[0]?.id;
	}
	onMount(load);

	let pack: IconPack | undefined;
	$: pack = packs.find((p) => p.id == selected);
	let icons: Icon[];
	$: {
		const query = search.trim().toLowerCase();
		icons = (pack?.icons ?? []).filter((i) => !query || i.name.toLowerCase().includes(query) || i.tags.some((tag) => tag.toLowerCase().includes(query)));
	}

	async function install() {
		const path = await open({ multiple: false, directory: false, filters: [{ name: $t("icon_packs.file"), extensions: ["streamDeckIconPack", "zip"] }] });
		if (!path) return;
		try {
			selected = await invoke("install_icon_pack", { path });
			await load();
		} catch (error: any) {
			message(String(error?.description ?? error), { title: $t("icon_packs.install.failed"), buttons: { ok: $t("dialog.ok") } });
		}
	}

	async function remove() {
		if (!pack) return;
		const confirmed = await ask($t("icon_packs.remove.confirm", { name: pack.name }), { title: $t("icon_packs.remove"), kind: "warning" });
		if (!confirmed) return;
		await invoke("remove_icon_pack", { id: pack.id });
		await load();
	}

	async function pick(icon: Icon) {
		const url = getWebserverUrl(icon.path);
		onPick((await resizeImage(url)) ?? url);
		onClose();
	}
</script>

<svelte:window
	on:keydown|capture={(event) => {
		if (event.key == "Escape") {
			event.stopPropagation();
			onClose();
		}
	}}
/>

<div
	class="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 flex flex-col w-[36rem] max-h-[30rem] p-3 text-neutral-300 bg-neutral-700 border border-neutral-600 rounded-lg z-20"
	role="dialog"
	aria-label={$t("icon_packs.title")}
>
	<div class="flex flex-row items-center space-x-2">
		<h2 class="font-semibold text-lg">{$t("icon_packs.title")}</h2>
		<div class="select-wrapper grow">
			<select class="w-full bg-neutral-600! border-neutral-500!" bind:value={selected} aria-label={$t("icon_packs.pack")} disabled={!packs.length}>
				{#each packs as p}
					<option value={p.id}>{p.name}{p.author ? ` (${p.author})` : ""}</option>
				{/each}
			</select>
		</div>
		{#if pack}
			<button on:click={remove} title={$t("icon_packs.remove")} aria-label={$t("icon_packs.remove")}>
				<Trash size="20" class="text-neutral-400" />
			</button>
		{/if}
		<button class="px-2 py-1 text-sm bg-neutral-600 hover:bg-neutral-500 transition-colors border border-neutral-500 rounded-lg" on:click={install}>
			{$t("icon_packs.install")}
		</button>
		<button class="ml-1 text-xl" on:click={onClose} aria-label={$t("settings.close")}>✕</button>
	</div>

	{#if pack}
		<input
			bind:value={search}
			placeholder={$t("icon_packs.search")}
			class="mt-2 px-2 py-1 text-neutral-300 bg-neutral-600 border border-neutral-500 rounded-lg"
			aria-label={$t("icon_packs.search")}
		/>
		<div class="grid grid-cols-6 gap-2 mt-2 overflow-auto">
			{#each icons as icon (icon.path)}
				<button
					class="flex flex-col items-center p-1 hover:bg-neutral-600 transition-colors rounded-lg"
					on:click={() => pick(icon)}
					title={[icon.name, ...icon.tags].join(", ")}
				>
					<img src={getWebserverUrl(icon.path)} alt={icon.name} class="w-16 h-16 rounded-md bg-black" loading="lazy" />
					<span class="mt-1 w-full text-xs text-center truncate">{icon.name}</span>
				</button>
			{:else}
				<p class="col-span-6 py-6 text-center text-neutral-400">{$t("icon_packs.no_match")}</p>
			{/each}
		</div>
	{:else}
		<p class="py-6 text-center text-neutral-400">{$t("icon_packs.none")}</p>
	{/if}
</div>
