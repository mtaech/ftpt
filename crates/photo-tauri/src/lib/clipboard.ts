// 剪贴板操作封装：预览主图右键菜单与胶片条缩略图右键菜单共用。
// 复制图片走 copyImageToClipboard（全尺寸 RGBA），复制路径走 copyTextToClipboard。
import { copyImageToClipboard, copyTextToClipboard } from '@/lib/ipc'
import { useRecognitionStore } from '@/stores/recognition'

/** 复制图片：成功/失败给状态栏提示（复用 recognition.setNotice 的瞬态提示通道） */
export async function copyImage(path: string) {
  const recognition = useRecognitionStore()
  try {
    await copyImageToClipboard(path)
    recognition.setNotice('已复制图片到剪贴板')
  } catch (e) {
    recognition.setNotice('复制图片失败：' + String(e))
  }
}

/** 复制文本（如文件绝对路径）：成功/失败给状态栏提示 */
export async function copyText(text: string) {
  const recognition = useRecognitionStore()
  try {
    await copyTextToClipboard(text)
    recognition.setNotice('已复制路径到剪贴板')
  } catch (e) {
    recognition.setNotice('复制路径失败：' + String(e))
  }
}
