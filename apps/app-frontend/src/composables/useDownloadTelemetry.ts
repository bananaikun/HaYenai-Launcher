import { onScopeDispose, type Ref, shallowRef } from 'vue'

import type { InstallJobSnapshot } from '@/helpers/install'

type DownloadTelemetry = Pick<
    InstallJobSnapshot['summary'],
    'speed_bytes_per_second' | 'eta_seconds'
>

type TelemetryJob = Pick<InstallJobSnapshot, 'job_id' | 'status'> & {
    summary: DownloadTelemetry
}

/** Samples display metrics without throttling download progress or scheduling. */
export function useDownloadTelemetry(jobs: Readonly<Ref<TelemetryJob[]>>) {
    function sample() {
        return new Map<string, DownloadTelemetry>(
            jobs.value
                .filter((job) => job.status === 'running')
                .map((job) => [
                    job.job_id,
                    {
                        speed_bytes_per_second: job.summary.speed_bytes_per_second,
                        eta_seconds: job.summary.eta_seconds,
                    },
                ]),
        )
    }

    const telemetry = shallowRef(sample())
    const timer = setInterval(() => {
        const next = sample()
        // 没有下载在跑时别每秒换一个新 Map：shallowRef 身份一变，整个下载列表
        // 就重渲染一遍。
        if (next.size === 0 && telemetry.value.size === 0) return
        if (
            next.size === telemetry.value.size &&
            [...next].every(([key, value]) => {
                const current = telemetry.value.get(key)
                return (
                    current?.speed_bytes_per_second === value.speed_bytes_per_second &&
                    current?.eta_seconds === value.eta_seconds
                )
            })
        ) {
            return
        }
        telemetry.value = next
    }, 1000)
    onScopeDispose(() => clearInterval(timer))

    return (job?: TelemetryJob): DownloadTelemetry | undefined => {
        if (!job || job.status !== 'running') return undefined
        const sampled = telemetry.value.get(job.job_id)
        if (!sampled) return undefined
        return {
            speed_bytes_per_second:
                job.summary.speed_bytes_per_second == null ? null : sampled.speed_bytes_per_second,
            eta_seconds: job.summary.eta_seconds == null ? null : sampled.eta_seconds,
        }
    }
}
