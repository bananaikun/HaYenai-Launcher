import { onScopeDispose, ref } from 'vue'

import { trackEvent } from '@/helpers/analytics'
import { get_jre, test_jre } from '@/helpers/jre.js'

export default function useJavaTest() {
    const testingJava = ref(false)
    const javaTestResult = ref<boolean | null>(null)
    let testDebounceTimer: ReturnType<typeof setTimeout> | null = null
    // 每次测试递增：慢的旧结果回来时不能覆盖新结果，也不能把还在跑的标志关掉。
    let runToken = 0

    async function runJavaTest(path: string, version: number | null, track = true) {
        if (testDebounceTimer) {
            clearTimeout(testDebounceTimer)
            testDebounceTimer = null
        }
        const token = ++runToken
        if (!path) {
            javaTestResult.value = null
            return
        }
        testingJava.value = true
        try {
            if (version == null) {
                const found = !!(await get_jre(path))
                if (token === runToken) javaTestResult.value = found
            } else {
                const ok = await test_jre(path, version)
                if (token === runToken) javaTestResult.value = ok
            }
        } catch {
            if (token === runToken) javaTestResult.value = false
        }
        if (token !== runToken) return
        testingJava.value = false

        if (track) {
            trackEvent('JavaTest', { path, success: javaTestResult.value })
        }
    }

    function testJavaInstallationDebounced(path: string, version: number | null, delay = 600) {
        if (testDebounceTimer) clearTimeout(testDebounceTimer)
        if (!path) {
            javaTestResult.value = null
            return
        }
        testDebounceTimer = setTimeout(() => runJavaTest(path, version, false), delay)
    }

    async function testJavaInstallation(path: string, version: number | null, track = false) {
        await runJavaTest(path, version, track)
    }

    onScopeDispose(() => {
        if (testDebounceTimer) {
            clearTimeout(testDebounceTimer)
            testDebounceTimer = null
        }
        runToken += 1
    })

    return {
        testingJava,
        javaTestResult,
        testJavaInstallationDebounced,
        testJavaInstallation,
    }
}
