"use client"

import React, { useState, useEffect, useRef } from "react"
import { motion, AnimatePresence } from "framer-motion"
import { Button } from "@/components/ui/button"
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar"
import { Badge } from "@/components/ui/badge"
import { Loader } from "@/components/ui/loader"
import {
  MessageSquare,
  Calendar as CalendarIcon,
  Search,
  Users,
  User,
  RefreshCw,
  Download,
  X,
  ChevronLeft,
  ChevronRight,
  ExternalLink,
  Send,
  AtSign,
  ImagePlus,
} from "lucide-react"
import { format } from "date-fns"
import { cn } from "@/lib/utils"
import { useStreamSearch, type SearchProgress } from "@/lib/useStreamSearch"
import { canLoadNextMessagePage } from "@/lib/message-preview-pagination"

interface MessagePreviewModalProps {
  open: boolean
  onClose: () => void
  chat: {
    type: 'group' | 'friend'
    id: string
    name: string
    peer: { chatType: number, peerUid: string, peerUin?: string }
  } | null
  onExport?: (peer: any, timeRange?: { startTime?: number, endTime?: number }) => void
  /** 是否展示发送消息输入栏（独立查看模式下不展示）。 */
  canSend?: boolean
}

interface Message {
  msgId: string
  msgSeq: number
  msgTime: number
  senderUid: string
  senderUin: string
  elements: any[]
  peerUin: string
  chatType: number
  sendType: number
  subMsgType: number
  sendMemberName?: string
  sendNickName?: string
}

interface MessagePreviewResponse {
  messages: Message[]
  totalCount: number
  currentPage: number
  totalPages: number
  hasNext: boolean
  fetchedAt: string
}

function marketFaceUrls(emojiId: string): { primary: string, fallback: string } | null {
  if (!/^[A-Za-z0-9_-]{2,}$/.test(emojiId)) return null
  const prefix = emojiId.slice(0, 2)
  const base = `https://gxh.vip.qq.com/club/item/parcel/item/${prefix}/${emojiId}/raw300`
  return { primary: `${base}.gif`, fallback: `${base}.png` }
}

export function MessagePreviewModal({ open, onClose, chat, onExport, canSend = false }: MessagePreviewModalProps) {
  const [messages, setMessages] = useState<Message[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [currentPage, setCurrentPage] = useState(1)
  const [totalPages, setTotalPages] = useState(1)
  const [totalCount, setTotalCount] = useState(0)
  const [hasNext, setHasNext] = useState(false)
  const [searchQuery, setSearchQuery] = useState("")
  const [startDate, setStartDate] = useState("")
  const [endDate, setEndDate] = useState("")
  const [timeRangeError, setTimeRangeError] = useState<string | null>(null)
  const [useStreamMode, setUseStreamMode] = useState(false)
  const [searchProgress, setSearchProgress] = useState("")
  const [currentFilter, setCurrentFilter] = useState<{ startTime: number; endTime: number } | null>(null)
  const [draft, setDraft] = useState("")
  const [sending, setSending] = useState(false)
  const [sendError, setSendError] = useState<string | null>(null)
  const [pendingImage, setPendingImage] = useState<{ dataUrl: string; name: string } | null>(null)
  const [mentions, setMentions] = useState<{ uin: string; name: string }[]>([])
  const [mentionPickerOpen, setMentionPickerOpen] = useState(false)
  const [members, setMembers] = useState<any[] | null>(null)
  const [membersLoading, setMembersLoading] = useState(false)
  const [memberSearch, setMemberSearch] = useState("")
  const imageInputRef = useRef<HTMLInputElement>(null)
  const silentFetchRef = useRef(false)
  
  const MESSAGES_PER_PAGE = 50
  
  const streamSearch = useStreamSearch({
    onProgress: (progress: SearchProgress) => {
      if (progress.status === 'searching') {
        setSearchProgress(`搜索中... ${progress.matchedCount} 条匹配`)
        if (progress.matchedCount > 0) setLoading(false)
      } else if (progress.status === 'completed') {
        setSearchProgress(`找到 ${progress.matchedCount} 条`)
        setLoading(false)
      }
    },
    onComplete: () => setLoading(false),
    onError: (err: string) => {
      setError(err)
      setLoading(false)
      setSearchProgress("")
    }
  })

  const validateTimeRange = (startValue: string, endValue: string): boolean => {
    if (startValue && endValue) {
      if (new Date(endValue) < new Date(startValue)) {
        setTimeRangeError('结束日期不能早于起始日期')
        return false
      }
    }
    setTimeRangeError(null)
    return true
  }

  const fetchMessages = async (page: number, filter?: any, silent = false) => {
    if (!chat) return
    if (!silent) setLoading(true)
    setError(null)

    try {
      let finalFilter = currentFilter
      if (!finalFilter) {
        finalFilter = { startTime: filter?.startTime || 0, endTime: filter?.endTime || Date.now() }
        setCurrentFilter(finalFilter)
      } else if (filter) {
        finalFilter = {
          startTime: filter.startTime ?? finalFilter.startTime,
          endTime: filter.endTime ?? finalFilter.endTime
        }
        setCurrentFilter(finalFilter)
      }

      const response = await fetch('/api/messages/fetch', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ peer: chat.peer, page, limit: MESSAGES_PER_PAGE, filter: finalFilter })
      })

      if (!response.ok) throw new Error(`HTTP error! status: ${response.status}`)
      const result = await response.json()
      if (!result.success) throw new Error(result.error?.message || '获取消息失败')

      const data: MessagePreviewResponse = result.data
      setMessages(data.messages || [])
      setTotalCount(data.totalCount || 0)
      setTotalPages(data.totalPages || 1)
      setHasNext(data.hasNext || false)
    } catch (err) {
      if (!silent) setError(err instanceof Error ? err.message : '获取消息失败')
    } finally {
      if (!silent) setLoading(false)
    }
  }

  const handleSearch = () => {
    if (!searchQuery.trim()) { handleTimeRangeChange(); return }
    if (!chat) return
    
    setUseStreamMode(true)
    setLoading(true)
    setMessages([])
    setSearchProgress("连接中...")
    
    streamSearch.startSearch({
      peer: chat.peer,
      filter: {
        startTime: startDate ? new Date(startDate).getTime() : 0,
        endTime: endDate ? (new Date(endDate).getTime() + 86400000 - 1) : Date.now()
      },
      searchQuery: searchQuery.trim()
    })
  }
  
  const handleTimeRangeChange = () => {
    if (!validateTimeRange(startDate, endDate) || (!startDate && !endDate)) return
    const filter = {
      startTime: startDate ? new Date(startDate).getTime() : 0,
      endTime: endDate ? new Date(endDate).getTime() + 86400000 - 1 : Date.now()
    }
    setCurrentPage(1)
    setUseStreamMode(false)
    setCurrentFilter(filter)
    fetchMessages(1, filter)
  }
  
  useEffect(() => {
    if (useStreamMode && streamSearch.results.length > 0) {
      const sorted = [...streamSearch.results].sort((a, b) => Number(b.msgTime) - Number(a.msgTime))
      setMessages(sorted)
      setTotalCount(sorted.length)
      setTotalPages(1)
      setCurrentPage(1)
    }
  }, [streamSearch.results, useStreamMode])

  const handleExportWithTimeRange = () => {
    if (!validateTimeRange(startDate, endDate)) return
    if (onExport && chat) {
      onExport(chat.peer, {
        startTime: startDate ? Math.floor(new Date(startDate).getTime() / 1000) : undefined,
        endTime: endDate ? Math.floor((new Date(endDate).getTime() + 86400000 - 1) / 1000) : undefined
      })
      onClose()
    }
  }

  const loadMembers = async () => {
    if (!chat || chat.type !== 'group' || members !== null || membersLoading) return
    setMembersLoading(true)
    try {
      const response = await fetch(`/api/groups/${encodeURIComponent(chat.id)}/members`)
      const result = await response.json()
      setMembers(result.success ? (result.data || []) : [])
    } catch {
      setMembers([])
    } finally {
      setMembersLoading(false)
    }
  }

  const memberDisplayName = (member: any): string =>
    String(member?.cardName || member?.card || member?.nick || member?.uin || "成员")

  const filteredMembers = (members || []).filter((member) => {
    const query = memberSearch.trim().toLowerCase()
    if (!query) return true
    return (
      memberDisplayName(member).toLowerCase().includes(query) ||
      String(member?.uin || "").includes(query)
    )
  }).slice(0, 50)

  const handlePickImage = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    e.target.value = ""
    if (!file) return
    if (file.size > 20 * 1024 * 1024) {
      setSendError("图片过大（最大 20MB）")
      return
    }
    const reader = new FileReader()
    reader.onload = () => {
      setPendingImage({ dataUrl: String(reader.result), name: file.name })
      setSendError(null)
    }
    reader.onerror = () => setSendError("读取图片失败")
    reader.readAsDataURL(file)
  }

  const canSubmit = !!draft.trim() || !!pendingImage || mentions.length > 0

  const handleSend = async () => {
    if (!chat || sending || !canSubmit) return

    setSending(true)
    setSendError(null)
    try {
      const payload: any = { peer: chat.peer, text: draft.trim() }
      if (pendingImage) {
        payload.image = { data: pendingImage.dataUrl, name: pendingImage.name }
      }
      if (chat.type === 'group' && mentions.length > 0) {
        payload.mentions = mentions.map((m) => m.uin)
      }
      const response = await fetch('/api/messages/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
      })
      if (!response.ok) throw new Error(`HTTP error! status: ${response.status}`)
      const result = await response.json()
      if (!result.success) throw new Error(result.error?.message || '发送失败')

      setDraft("")
      setPendingImage(null)
      setMentions([])
      // NapCat 落库有短暂延迟，稍等再刷新到第一页（顺带清掉日期筛选）。
      setTimeout(() => {
        const filter = { startTime: 0, endTime: Date.now() }
        setCurrentPage(1)
        setCurrentFilter(filter)
        fetchMessages(1, filter)
      }, 800)
    } catch (err) {
      setSendError(err instanceof Error ? err.message : '发送失败')
    } finally {
      setSending(false)
    }
  }

  const formatMessageElements = (elements: any[], msg?: any): React.ReactNode[] => {
    if (!elements?.length) return [<span key="empty" className="text-muted-foreground/50 italic">无文本内容</span>]
    const nodes: React.ReactNode[] = []
    let hasContent = false
    for (let i = 0; i < elements.length; i++) {
      const el = elements[i]
      if (el.replyElement) {
        const replyText = el.replyElement.sourceMsgTextElems
          ?.map((e: any) => e.textElemContent || '').join('').trim()
          || el.replyElement.sourceMsgText || ''
        nodes.push(
          <span key={`reply-${i}`} className="inline-flex items-center gap-1 text-xs text-muted-foreground/60 bg-black/[0.04] dark:bg-white/[0.06] rounded px-1.5 py-0.5 mr-1 align-middle">
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="shrink-0"><polyline points="9 17 4 12 9 7"/><path d="M20 18v-2a4 4 0 0 0-4-4H4"/></svg>
            {replyText ? <span className="truncate max-w-[120px]">{replyText}</span> : '回复'}
          </span>
        )
        hasContent = true
      } else if (el.textElement?.content) {
        const text = el.textElement.content as string
        const urlRegex = /(https?:\/\/[^\s<>"']+)/g
        const parts = text.split(urlRegex)
        if (parts.length > 1) {
          const textNodes: React.ReactNode[] = []
          parts.forEach((part, pi) => {
            if (urlRegex.test(part) || part.match(/^https?:\/\//)) {
              textNodes.push(
                <a key={`link-${i}-${pi}`} href={part} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-0.5 text-foreground decoration-dashed underline underline-offset-2 hover:text-blue-600 transition-colors">
                  <span>{part}</span>
                  <ExternalLink className="w-3 h-3 shrink-0 opacity-50" />
                </a>
              )
            } else if (part) {
              textNodes.push(<span key={`t-${i}-${pi}`}>{part}</span>)
            }
          })
          nodes.push(<span key={`text-${i}`}>{textNodes}</span>)
        } else {
          nodes.push(<span key={`text-${i}`}>{text}</span>)
        }
        hasContent = true
      } else if (el.picElement) {
        const pic = el.picElement
        const thumbRaw = pic.thumbPath
        // NTQQ 的 thumbPath 是 { "0": path } 形式的对象而非字符串。
        const thumbPath =
          typeof thumbRaw === 'string' && thumbRaw
            ? thumbRaw
            : thumbRaw && typeof thumbRaw === 'object'
              ? (Object.values(thumbRaw)[0] as string) || ''
              : ''
        const localPath = pic.sourcePath || thumbPath
        const remoteUrl = pic.originImageUrl || ''
        // 本地缓存路径经服务端鉴权端点流式返回；附上消息定位参数后，
        // 服务端可在 NTQQ 懒下载未命中时按需拉取。远程 URL 仅在无本地路径
        // 时用，并作为本地文件缺失时的兜底（QQ CDN 链接可能已过期）。
        const picUrl = (() => {
          if (!localPath) return remoteUrl
          const params = new URLSearchParams({ path: localPath })
          if (thumbPath && thumbPath !== localPath) params.set('thumbPath', thumbPath)
          if (msg?.msgId != null) params.set('msgId', String(msg.msgId))
          if (msg?.chatType != null) params.set('chatType', String(msg.chatType))
          if (msg?.peerUid) params.set('peerUid', msg.peerUid)
          if (el.elementId != null) params.set('elementId', String(el.elementId))
          return `/api/media/local?${params.toString()}`
        })()
        const w = pic.picWidth || 200
        const h = pic.picHeight || 200
        const displayW = Math.min(w, 200)
        const displayH = Math.round(displayW * (h / w))
        nodes.push(
          <span key={`pic-${i}`} className="inline-block align-middle my-0.5">
            <img
              src={picUrl}
              alt="图片"
              width={displayW}
              height={displayH}
              className="rounded-lg object-cover max-w-[200px] max-h-[150px] bg-muted"
              loading="lazy"
              onError={(e) => {
                const image = e.target as HTMLImageElement
                if (remoteUrl && image.dataset.fallbackTried !== 'true') {
                  image.dataset.fallbackTried = 'true'
                  image.src = remoteUrl
                  return
                }
                image.style.display = 'none'
                image.insertAdjacentHTML('afterend', '<span class="inline-flex items-center gap-1 text-xs text-muted-foreground/70">[\u56fe\u7247]</span>')
              }}
            />
          </span>
        )
        hasContent = true
      } else if (el.marketFaceElement) {
        const faceName = el.marketFaceElement.faceName || '表情'
        const emojiId = el.marketFaceElement.emojiId || ''
        const stickerUrls = marketFaceUrls(emojiId)
        if (stickerUrls) {
          nodes.push(
            <span key={`mface-${i}`} className="inline-block align-middle my-0.5">
              <img
                src={stickerUrls.primary}
                alt={faceName}
                className="w-16 h-16 object-contain"
                loading="lazy"
                onError={(e) => {
                  const image = e.currentTarget
                  if (image.dataset.fallbackTried !== 'true') {
                    image.dataset.fallbackTried = 'true'
                    image.src = stickerUrls.fallback
                    return
                  }
                  image.style.display = 'none'
                  const fallback = document.createElement('span')
                  fallback.className = 'text-muted-foreground/70 text-sm'
                  fallback.textContent = faceName
                  image.insertAdjacentElement('afterend', fallback)
                }}
              />
            </span>
          )
        } else {
          nodes.push(
            <span key={`mface-${i}`} className="text-muted-foreground/70 text-sm">{faceName}</span>
          )
        }
        hasContent = true
      } else if (el.faceElement) {
        const faceId = el.faceElement.faceIndex ?? el.faceElement.faceType ?? ''
        const qqFaceUrl = faceId ? `https://res.qlogo.cn/qqface/${faceId}/100` : ''
        if (qqFaceUrl && faceId) {
          nodes.push(
            <img key={`face-${i}`} src={qqFaceUrl} alt={`表情#${faceId}`} className="inline-block w-5 h-5 align-middle" loading="lazy" onError={(e) => { (e.target as HTMLImageElement).outerHTML = `<span class="text-muted-foreground/70">[\u8868\u60c5 #${faceId}]</span>` }} />
          )
        } else {
          nodes.push(
            <span key={`face-${i}`} className="text-muted-foreground/70">[表情{faceId ? ` #${faceId}` : ''}]</span>
          )
        }
        hasContent = true
      } else if (el.pttElement) {
        const duration = el.pttElement.duration ? `${el.pttElement.duration}"` : ''
        nodes.push(
          <span key={`ptt-${i}`} className="inline-flex items-center gap-1 text-xs text-muted-foreground/70 align-middle">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="shrink-0"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" x2="12" y1="19" y2="22"/></svg>
            语音{duration ? ` ${duration}` : ''}
          </span>
        )
        hasContent = true
      } else if (el.videoElement) {
        nodes.push(
          <span key={`video-${i}`} className="inline-flex items-center gap-1 text-xs text-muted-foreground/70 align-middle">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="shrink-0"><path d="m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5"/><rect width="14" height="12" x="2" y="6" rx="2"/></svg>
            视频
          </span>
        )
        hasContent = true
      } else if (el.fileElement) {
        const fileName = el.fileElement.fileName || '文件'
        nodes.push(
          <span key={`file-${i}`} className="inline-flex items-center gap-1 text-xs text-muted-foreground/70 align-middle">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="shrink-0"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/></svg>
            {fileName}
          </span>
        )
        hasContent = true
      } else if (el.arkElement) {
        nodes.push(<span key={`ark-${i}`} className="text-muted-foreground/70">[卡片消息]</span>)
        hasContent = true
      } else if (el.grayTipElement) {
        const tip = el.grayTipElement.xmlElement?.content || el.grayTipElement.jsonGrayTipElement?.jsonStr || ''
        nodes.push(<span key={`tip-${i}`} className="text-muted-foreground/50 italic text-xs">{tip || '[系统提示]'}</span>)
        hasContent = true
      } else if (el.multiForwardMsgElement) {
        nodes.push(<span key={`fwd-${i}`} className="text-muted-foreground/70">[合并转发]</span>)
        hasContent = true
      } else if (el.faceBubbleElement) {
        const text = el.faceBubbleElement?.richNameElement?.textElement?.content || '表情气泡'
        nodes.push(<span key={`fb-${i}`} className="text-muted-foreground/70">[{text}]</span>)
        hasContent = true
      }
    }
    if (!hasContent) return [<span key="empty" className="text-muted-foreground/50 italic">无文本内容</span>]
    return nodes
  }

  useEffect(() => {
    if (open && chat) {
      setMessages([])
      setCurrentPage(1)
      setTotalPages(1)
      setTotalCount(0)
      setSearchQuery("")
      setStartDate("")
      setEndDate("")
      setTimeRangeError(null)
      setError(null)
      setCurrentFilter(null)
      setDraft("")
      setSendError(null)
      setPendingImage(null)
      setMentions([])
      setMentionPickerOpen(false)
      setMembers(null)
      setMemberSearch("")
      fetchMessages(1)
    }
  }, [open, chat])

  // 会话模式下每 4s 静默拉一次最新消息。仅在第一页、无搜索/流式
  // 结果、未固定结束日期时轮询，避免打断翻页和筛选浏览。
  useEffect(() => {
    if (!open || !canSend || !chat) return
    const timer = setInterval(() => {
      if (typeof document !== 'undefined' && document.visibilityState !== 'visible') return
      if (silentFetchRef.current || loading || useStreamMode || searchQuery.trim() || currentPage !== 1 || endDate) return
      silentFetchRef.current = true
      const filter = {
        startTime: startDate ? new Date(startDate).getTime() : 0,
        endTime: Date.now()
      }
      fetchMessages(1, filter, true).finally(() => {
        silentFetchRef.current = false
      })
    }, 4000)
    return () => clearInterval(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, canSend, chat, loading, useStreamMode, searchQuery, currentPage, startDate, endDate])

  /**
   * Issue #300: 这个预览弹窗本身是 framer-motion 自定义模态，z-index 在更外层 Radix
   * Dialog 之上，但 Radix 在它自己的 `<body>` 上挂了 `pointer-events: none` 来禁用底
   * 层交互。结果就是任务向导里的 Dialog 还开着时打开预览，预览本身和它 fixed 子树
   * 也被一起禁掉了点击。
   *
   * 这里在弹窗显示期间强制把 body 的 pointer-events 改回 auto。关闭时只在当前 body
   * 仍是我们写入的 `auto` 时才回写之前的值；如果其它来源（比如 Radix 关闭 wizard 时
   * 同步清掉了它自己的 lock）已经动过，我们就不再覆盖，避免把陈旧的 'none' 写回去
   * 把整页锁死（Codex review on PR #400）。
   */
  useEffect(() => {
    if (!open) return
    if (typeof document === 'undefined') return
    const body = document.body
    const previous = body.style.pointerEvents
    body.style.pointerEvents = 'auto'
    return () => {
      if (body.style.pointerEvents === 'auto') {
        body.style.pointerEvents = previous
      }
    }
  }, [open])

  // Intercept Escape key so it only closes the preview modal, not the
  // underlying Radix Dialog (TaskWizard). We capture in the capture phase
  // to beat Radix's own keydown listener.
  useEffect(() => {
    if (!open) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        e.preventDefault()
        onClose()
      }
    }
    document.addEventListener('keydown', handler, true)
    return () => document.removeEventListener('keydown', handler, true)
  }, [open, onClose])

  if (!chat) return null

  return (
    <AnimatePresence>
      {open && (
      <>
      {/* Backdrop */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.15 }}
        className="fixed inset-0 bg-background/80 z-[110]"
        onClick={onClose}
      />

      {/* Fullscreen modal */}
      <motion.div
        initial={{ opacity: 0, scale: 0.97, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.97, y: 8 }}
        transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
        className="fixed inset-4 z-[111] flex flex-col bg-card rounded-2xl shadow-[0_24px_80px_rgba(0,0,0,0.12)] dark:shadow-[0_24px_80px_rgba(0,0,0,0.4)] overflow-hidden"
      >
        {/* Header */}
        <div className="px-6 py-4 flex items-center justify-between flex-shrink-0">
          <div className="flex items-center gap-3">
            <Avatar className="w-10 h-10 rounded-full">
              <AvatarImage src={chat.type === 'group' 
                ? `https://p.qlogo.cn/gh/${chat.id}/${chat.id}/40`
                : `https://q1.qlogo.cn/g?b=qq&nk=${chat.id}&s=40`
              } />
              <AvatarFallback className="bg-muted text-muted-foreground rounded-full">
                {chat.type === 'group' ? <Users className="w-4 h-4" /> : <User className="w-4 h-4" />}
              </AvatarFallback>
            </Avatar>
            <div>
              <h2 className="text-base font-semibold text-foreground leading-tight">{chat.name}</h2>
              <p className="text-xs text-muted-foreground">
                {chat.type === 'group' ? '群聊' : '好友'}
                <span aria-hidden className="mx-1 inline-block h-3 w-px translate-y-[2px] bg-current opacity-25" />
                {totalCount.toLocaleString()} 条消息
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="w-8 h-8 flex items-center justify-center text-muted-foreground hover:text-foreground hover:bg-black/[0.04] dark:hover:bg-white/[0.06] rounded-full transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Filters - single compact row */}
        <div className="px-6 py-3 flex items-center gap-3 flex-shrink-0 flex-wrap">
          {/* Time range */}
          <div className="flex items-center gap-2">
            <CalendarIcon className="w-3.5 h-3.5 text-muted-foreground" />
            <input
              type="date"
              value={startDate}
              onChange={(e) => { setStartDate(e.target.value); validateTimeRange(e.target.value, endDate) }}
              className="px-2.5 py-1 text-xs bg-transparent text-foreground rounded-full"
            />
            <span className="text-muted-foreground/40 text-xs">—</span>
            <input
              type="date"
              value={endDate}
              onChange={(e) => { setEndDate(e.target.value); validateTimeRange(startDate, e.target.value) }}
              className="px-2.5 py-1 text-xs bg-transparent text-foreground rounded-full"
            />
            <Button 
              variant="outline" 
              size="sm"
              onClick={handleTimeRangeChange}
              disabled={(!startDate && !endDate) || !!timeRangeError}
              className="rounded-full h-7 text-xs px-3"
            >
              筛选
            </Button>
            {(startDate || endDate) && (
              <button
                onClick={() => { setStartDate(""); setEndDate(""); setTimeRangeError(null); setCurrentFilter(null); fetchMessages(1) }}
                className="text-xs text-muted-foreground hover:text-foreground transition-colors"
              >
                清除
              </button>
            )}
            {timeRangeError && <span className="text-xs text-red-500">{timeRangeError}</span>}
          </div>

          {/* Divider */}
          <div className="w-px h-5 bg-black/[0.06] dark:bg-white/[0.06]" />

          {/* Search */}
          <div className="flex items-center gap-2 flex-1 min-w-[200px]">
            <div className="flex-1 flex items-center gap-2 px-3 py-1.5 rounded-full">
              <Search className="w-3.5 h-3.5 text-muted-foreground/60" />
              <input
                type="text"
                placeholder="搜索消息内容..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && handleSearch()}
                className="flex-1 text-xs bg-transparent outline-none text-foreground placeholder:text-muted-foreground/60"
              />
              {searchQuery && (
                <button onClick={() => { setSearchQuery(""); setUseStreamMode(false); setSearchProgress(""); fetchMessages(1) }}>
                  <X className="w-3.5 h-3.5 text-muted-foreground hover:text-foreground" />
                </button>
              )}
            </div>
            <Button size="sm" onClick={handleSearch} disabled={loading || !searchQuery.trim()} className="rounded-full h-7 text-xs px-3">
              {streamSearch.searching ? <Loader size={14} /> : '搜索'}
            </Button>
            {streamSearch.searching && (
              <Button size="sm" variant="ghost" onClick={() => streamSearch.cancelSearch()} className="rounded-full h-7 text-xs px-3">取消</Button>
            )}
            {searchProgress && <Badge variant="secondary" className="text-xs rounded-full">{searchProgress}</Badge>}
          </div>
        </div>

        {/* Messages */}
        <div className="flex-1 min-h-0 overflow-y-auto">
          <div className="max-w-4xl mx-auto px-6 py-4">
            {loading && (
              <div className="flex flex-col items-center justify-center py-20">
                <Loader size={20} className="text-muted-foreground/40 mb-3" />
                <p className="text-sm text-muted-foreground">加载中...</p>
              </div>
            )}

            {error && (
              <div className="flex flex-col items-center justify-center py-20">
                <p className="text-sm text-red-500 dark:text-red-400 mb-3">{error}</p>
                <Button variant="outline" size="sm" onClick={() => fetchMessages(1)} className="rounded-full">重试</Button>
              </div>
            )}

            {!loading && !error && messages.length === 0 && (
              <div className="flex flex-col items-center justify-center py-20">
                <p className="text-sm text-muted-foreground">{searchQuery ? '没有找到匹配的消息' : '暂无消息'}</p>
              </div>
            )}

            {!loading && !error && messages.length > 0 && (
              <div className="space-y-1">
                {messages.map((msg, idx) => {
                  const isFromSelf = msg.sendType === 1
                  return (
                    <div
                      key={`${msg.msgId}-${idx}`}
                      className={cn(
                        "flex gap-3 px-4 py-3 rounded-xl transition-colors hover:bg-black/[0.02] dark:hover:bg-white/[0.02]",
                        isFromSelf && "bg-black/[0.02] dark:bg-white/[0.02]"
                      )}
                    >
                      <Avatar className="w-8 h-8 flex-shrink-0 rounded-full">
                        <AvatarImage src={`https://q1.qlogo.cn/g?b=qq&nk=${msg.senderUin}&s=40`} />
                        <AvatarFallback className="text-xs bg-muted text-muted-foreground rounded-full">
                          {(msg.sendMemberName || msg.sendNickName || '?')[0]}
                        </AvatarFallback>
                      </Avatar>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2 mb-0.5">
                          <span className="text-sm font-medium text-foreground truncate">
                            {msg.sendMemberName || msg.sendNickName || `用户${msg.senderUin}`}
                          </span>
                          <span className="text-xs text-muted-foreground/50">
                            {format(new Date(msg.msgTime * 1000), 'MM-dd HH:mm')}
                          </span>
                        </div>
                        <p className="text-sm text-foreground/80 break-words leading-relaxed">
                          {formatMessageElements(msg.elements, msg)}
                        </p>
                      </div>
                    </div>
                  )
                })}
              </div>
            )}
          </div>
        </div>

        {/* Send composer */}
        {canSend && (
          <div className="relative px-6 py-3 border-t border-black/[0.06] dark:border-white/[0.06] flex-shrink-0">
            {sendError && <p className="text-xs text-red-500 mb-2">{sendError}</p>}

            {(mentions.length > 0 || pendingImage) && (
              <div className="flex items-center gap-1.5 flex-wrap mb-2">
                {mentions.map((mention) => (
                  <span
                    key={mention.uin}
                    className="inline-flex items-center gap-1 text-xs bg-black/[0.05] dark:bg-white/[0.08] rounded-full pl-2.5 pr-1 py-0.5"
                  >
                    @{mention.name}
                    <button
                      onClick={() => setMentions((prev) => prev.filter((m) => m.uin !== mention.uin))}
                      className="w-4 h-4 flex items-center justify-center text-muted-foreground hover:text-foreground rounded-full"
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </span>
                ))}
                {pendingImage && (
                  <span className="inline-flex items-center gap-1.5 text-xs bg-black/[0.05] dark:bg-white/[0.08] rounded-full pl-1 pr-1 py-0.5">
                    <img src={pendingImage.dataUrl} alt={pendingImage.name} className="w-6 h-6 rounded object-cover" />
                    <button
                      onClick={() => setPendingImage(null)}
                      className="w-4 h-4 flex items-center justify-center text-muted-foreground hover:text-foreground rounded-full"
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </span>
                )}
              </div>
            )}

            <div className="flex items-end gap-2">
              {chat.type === 'group' && (
                <button
                  onClick={() => {
                    const next = !mentionPickerOpen
                    setMentionPickerOpen(next)
                    if (next) loadMembers()
                  }}
                  title="提到群成员"
                  className="w-9 h-9 flex items-center justify-center text-muted-foreground hover:text-foreground hover:bg-black/[0.04] dark:hover:bg-white/[0.06] rounded-full transition-colors flex-shrink-0"
                >
                  <AtSign className="w-4 h-4" />
                </button>
              )}
              <button
                onClick={() => imageInputRef.current?.click()}
                title="发送图片"
                className="w-9 h-9 flex items-center justify-center text-muted-foreground hover:text-foreground hover:bg-black/[0.04] dark:hover:bg-white/[0.06] rounded-full transition-colors flex-shrink-0"
              >
                <ImagePlus className="w-4 h-4" />
              </button>
              <input
                ref={imageInputRef}
                type="file"
                accept="image/png,image/jpeg,image/gif,image/webp,image/bmp"
                className="hidden"
                onChange={handlePickImage}
              />
              <textarea
                value={draft}
                onChange={(e) => { setDraft(e.target.value); if (sendError) setSendError(null) }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                    e.preventDefault()
                    handleSend()
                  }
                }}
                placeholder={`发消息给 ${chat.name}（Enter 发送，Shift+Enter 换行）`}
                rows={2}
                disabled={sending}
                className="flex-1 resize-none text-sm text-foreground placeholder:text-muted-foreground/60 outline-none px-3 py-2 rounded-xl bg-black/[0.03] dark:bg-white/[0.04]"
              />
              <Button
                size="sm"
                onClick={handleSend}
                disabled={sending || !canSubmit}
                className="rounded-full h-9 px-4"
              >
                {sending ? <Loader size={14} className="mr-1.5" /> : <Send className="w-3.5 h-3.5 mr-1.5" />}
                发送
              </Button>
            </div>

            {mentionPickerOpen && chat.type === 'group' && (
              <div className="absolute bottom-full left-6 mb-2 w-72 rounded-xl border border-black/[0.08] dark:border-white/[0.1] bg-card shadow-[0_12px_40px_rgba(0,0,0,0.12)] dark:shadow-[0_12px_40px_rgba(0,0,0,0.4)] overflow-hidden z-10">
                <div className="p-2 border-b border-black/[0.06] dark:border-white/[0.06]">
                  <input
                    type="text"
                    value={memberSearch}
                    onChange={(e) => setMemberSearch(e.target.value)}
                    placeholder="搜索群成员..."
                    autoFocus
                    className="w-full px-2.5 py-1.5 text-xs bg-transparent text-foreground placeholder:text-muted-foreground/60 outline-none rounded-lg bg-black/[0.03] dark:bg-white/[0.04]"
                  />
                </div>
                <div className="max-h-56 overflow-y-auto p-1">
                  {membersLoading && (
                    <div className="flex items-center justify-center py-6">
                      <Loader size={14} className="text-muted-foreground/40" />
                    </div>
                  )}
                  {!membersLoading && members !== null && filteredMembers.length === 0 && (
                    <p className="text-xs text-muted-foreground text-center py-6">
                      {members.length === 0 ? '无法获取成员列表' : '没有匹配的成员'}
                    </p>
                  )}
                  {!membersLoading && filteredMembers.map((member) => {
                    const uin = String(member?.uin || "")
                    const name = memberDisplayName(member)
                    const selected = mentions.some((m) => m.uin === uin)
                    return (
                      <button
                        key={uin || member?.uid || name}
                        disabled={selected || !uin}
                        onClick={() => {
                          setMentions((prev) => [...prev, { uin, name }])
                          setMentionPickerOpen(false)
                          setMemberSearch("")
                        }}
                        className="w-full flex items-center gap-2 px-2.5 py-2 text-left rounded-lg hover:bg-black/[0.04] dark:hover:bg-white/[0.06] disabled:opacity-40 transition-colors"
                      >
                        <Avatar className="w-6 h-6 rounded-full">
                          <AvatarImage src={`https://q1.qlogo.cn/g?b=qq&nk=${uin}&s=40`} />
                          <AvatarFallback className="text-[10px] bg-muted text-muted-foreground rounded-full">
                            {name[0]}
                          </AvatarFallback>
                        </Avatar>
                        <span className="flex-1 text-xs text-foreground truncate">{name}</span>
                        {selected && <span className="text-[10px] text-muted-foreground">已选</span>}
                      </button>
                    )
                  })}
                </div>
              </div>
            )}
          </div>
        )}

        {/* Footer */}
        <div className="px-6 py-3 border-t border-black/[0.06] dark:border-white/[0.06] flex items-center justify-between flex-shrink-0">
          {/* Pagination */}
          {!useStreamMode && totalPages > 1 ? (
            <div className="flex items-center gap-1.5">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => { setCurrentPage(p => p - 1); fetchMessages(currentPage - 1) }}
                disabled={currentPage <= 1 || loading}
                className="rounded-full h-8 w-8 p-0"
              >
                <ChevronLeft className="w-4 h-4" />
              </Button>
              <span className="text-xs text-muted-foreground px-2">
                {currentPage} / {totalPages}
              </span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => { setCurrentPage(p => p + 1); fetchMessages(currentPage + 1) }}
                disabled={!canLoadNextMessagePage(currentPage, totalPages, hasNext) || loading}
                className="rounded-full h-8 w-8 p-0"
              >
                <ChevronRight className="w-4 h-4" />
              </Button>
            </div>
          ) : (
            <div />
          )}

          {/* Actions */}
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => fetchMessages(currentPage)}
              disabled={loading}
              className="rounded-full h-8 text-xs px-4"
            >
              {loading ? <Loader size={14} className="mr-1.5" /> : <RefreshCw className="w-3.5 h-3.5 mr-1.5" />}
              刷新
            </Button>
            {onExport && (
              <Button size="sm" onClick={handleExportWithTimeRange} className="rounded-full h-8 text-xs px-4">
                <Download className="w-3.5 h-3.5 mr-1.5" />
                导出
              </Button>
            )}
          </div>
        </div>
      </motion.div>
      </>
      )}
    </AnimatePresence>
  )
}
