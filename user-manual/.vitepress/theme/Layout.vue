<script setup lang="ts">
import DefaultTheme from 'vitepress/theme'
import { useData } from 'vitepress'

// channel/releaseTag are plain strings stashed on themeConfig by config.mts
// (not a documented VitePress field, just piggybacking on the object that's
// already serialized to the client).
const { theme } = useData()
const channel = (theme.value as { channel: string }).channel
const releaseTag = (theme.value as { releaseTag: string }).releaseTag
</script>

<template>
  <DefaultTheme.Layout>
    <template #layout-top>
      <div v-if="channel === 'nightly'" class="nightly-banner">
        You're reading docs for unreleased changes —
        <a href="https://cvlmtg.github.io/HUME/">switch to {{ releaseTag }}</a>.
      </div>
    </template>
  </DefaultTheme.Layout>
</template>
