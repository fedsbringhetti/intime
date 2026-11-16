<script lang="ts">
  import TabBar from './lib/components/TabBar.svelte'
  import UpdateBanner from './lib/components/UpdateBanner.svelte'
  import { navigation, startNavigation } from './lib/navigation.svelte'
  import Favorites from './screens/Favorites.svelte'
  import Home from './screens/Home.svelte'
  import Info from './screens/Info.svelte'
  import Near from './screens/Near.svelte'
  import Notifications from './screens/Notifications.svelte'
  import NotFound from './screens/NotFound.svelte'
  import Search from './screens/Search.svelte'
  import Stop from './screens/Stop.svelte'
  import Tracking from './screens/Tracking.svelte'

  // Runs once the app is on the page; the returned function runs when it is removed.
  $effect(() => startNavigation())

  // The design page is loaded on demand and only in development: in a production build
  // `import.meta.env.DEV` is `false`, so the import is dropped and the file is not shipped.
  const designPage = import.meta.env.DEV ? () => import('./screens/DesignPreview.svelte') : null
</script>

<main>
  {#if navigation.route.name === 'home'}
    <Home />
  {:else if navigation.route.name === 'search'}
    <Search query={navigation.route.query} />
  {:else if navigation.route.name === 'stop'}
    <Stop stop={navigation.route.stop} lines={navigation.route.lines} />
  {:else if navigation.route.name === 'near'}
    <Near />
  {:else if navigation.route.name === 'favorites'}
    <Favorites />
  {:else if navigation.route.name === 'notifications'}
    <Notifications />
  {:else if navigation.route.name === 'tracking'}
    {#key navigation.route.id}
      <Tracking id={navigation.route.id} />
    {/key}
  {:else if navigation.route.name === 'info'}
    <Info />
  {:else if navigation.route.name === 'design' && designPage}
    {#await designPage() then page}
      <page.default />
    {/await}
  {:else}
    <NotFound />
  {/if}
</main>
<UpdateBanner />
<TabBar />

<style>
  main {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 640px;
    margin: 0 auto;
    padding: max(12px, env(safe-area-inset-top)) var(--gutter)
      calc(var(--tab-bar-height) + env(safe-area-inset-bottom) + 24px);
    padding-left: max(var(--gutter), env(safe-area-inset-left));
    padding-right: max(var(--gutter), env(safe-area-inset-right));
  }
</style>
