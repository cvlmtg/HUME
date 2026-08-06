<script setup lang="ts">
import DefaultTheme from 'vitepress/theme'
import { useData } from 'vitepress'

// channel/releaseTag are plain strings stashed on themeConfig by config.mts
// (not a documented VitePress field, just piggybacking on the object that's
// already serialized to the client).
const { theme } = useData()
const channel = (theme.value as { channel: string }).channel
const releaseTag = (theme.value as { releaseTag: string }).releaseTag
const releaseLink = (theme.value as { releaseLink: string }).releaseLink
</script>

<template>
  <DefaultTheme.Layout>
    <template #layout-top>
      <div v-if="channel === 'nightly'" class="nightly-banner">
        <!-- target="_self" stops VitePress's router from hijacking this as an
             SPA route change — see the matching comment in config.mts. -->
        You're reading docs for unreleased changes —&nbsp;
        <a :href="releaseLink" target="_self">switch to {{ releaseTag }}</a>.
      </div>
    </template>
  </DefaultTheme.Layout>
</template>
