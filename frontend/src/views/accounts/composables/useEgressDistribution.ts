import type { EgressDistribution } from '@/api'
import { onMounted, shallowRef } from 'vue'
import { getEgressDistribution } from '@/api'

/** 账号页出口分布：挂载时加载一次只读分组事实，不参与账号增删改流程。 */
export function useEgressDistribution() {
  const distribution = shallowRef<EgressDistribution | null>(null)
  const loading = shallowRef(false)

  async function loadEgressDistribution() {
    if (loading.value)
      return
    loading.value = true
    try {
      distribution.value = await getEgressDistribution()
    }
    catch {}
    finally {
      loading.value = false
    }
  }

  onMounted(() => void loadEgressDistribution())
  return { distribution, loading, loadEgressDistribution }
}
