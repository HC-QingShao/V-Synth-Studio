import {useSyncExternalStore} from 'react'
import {getConfig, onConfigChange, saveConfig} from './config'

export type UiLanguage = 'zh-CN' | 'en-US' | 'ja-JP'

const DICT: Record<string, Record<UiLanguage, string>> = {
  '总览': {'zh-CN':'总览','en-US':'Dashboard','ja-JP':'概要'},
  '环境检测与常用入口': {'zh-CN':'环境检测与常用入口','en-US':'Environment checks and quick actions','ja-JP':'環境チェックとクイック操作'},
  '工程转换': {'zh-CN':'工程转换','en-US':'Project Converter','ja-JP':'プロジェクト変換'},
  '把工程转到另一个编辑器': {'zh-CN':'把工程转到另一个编辑器','en-US':'Convert projects between editors','ja-JP':'別のエディター向けにプロジェクトを変換'},
  '视频解析': {'zh-CN':'视频解析','en-US':'Video Downloader','ja-JP':'動画解析'},
  'B 站 / YouTube 等平台的 MV 下载': {'zh-CN':'B 站 / YouTube 等平台的 MV 下载','en-US':'Download MV from Bilibili, YouTube and more','ja-JP':'Bilibili・YouTube などから MV をダウンロード'},
  '音轨分离': {'zh-CN':'音轨分离','en-US':'Stem Separation','ja-JP':'ステム分離'},
  '人声转 MIDI': {'zh-CN':'人声转 MIDI','en-US':'Vocal to MIDI','ja-JP':'ボーカル→MIDI'},
  '音频工具': {'zh-CN':'音频工具','en-US':'Audio Tools','ja-JP':'オーディオツール'},
  '网易云专栏': {'zh-CN':'网易云专栏','en-US':'Netease Music','ja-JP':'NetEase Music'},
  '文字 PV': {'zh-CN':'文字 PV','en-US':'Lyric PV','ja-JP':'歌詞 PV'},
  '资源库': {'zh-CN':'资源库','en-US':'Resources','ja-JP':'リソース'},
  '设置': {'zh-CN':'设置','en-US':'Settings','ja-JP':'設定'},
  '外观': {'zh-CN':'外观','en-US':'Appearance','ja-JP':'外観'},
  '壁纸': {'zh-CN':'壁纸','en-US':'Wallpaper','ja-JP':'壁紙'},
  '路径': {'zh-CN':'路径','en-US':'Paths','ja-JP':'パス'},
  '外部工具': {'zh-CN':'外部工具','en-US':'External Tools','ja-JP':'外部ツール'},
  '关于': {'zh-CN':'关于','en-US':'About','ja-JP':'概要'},
  '工作台': {'zh-CN':'工作台','en-US':'Workspace','ja-JP':'ワークスペース'},
  '素材获取': {'zh-CN':'素材获取','en-US':'Assets','ja-JP':'素材'},
  '系统': {'zh-CN':'系统','en-US':'System','ja-JP':'システム'},
  '设置分节': {'zh-CN':'设置分节','en-US':'Settings sections','ja-JP':'設定セクション'},
  '主题': {'zh-CN':'主题','en-US':'Theme','ja-JP':'テーマ'},
  '跟随系统': {'zh-CN':'跟随系统','en-US':'System','ja-JP':'システム'},
  '明亮': {'zh-CN':'明亮','en-US':'Light','ja-JP':'ライト'},
  '黑暗': {'zh-CN':'黑暗','en-US':'Dark','ja-JP':'ダーク'},
  '整套界面的配色': {'zh-CN':'整套界面的配色','en-US':'Color scheme for the entire interface','ja-JP':'インターフェース全体の配色'},
  '语言': {'zh-CN':'语言','en-US':'Language','ja-JP':'言語'},
  '简体中文': {'zh-CN':'简体中文','en-US':'Simplified Chinese','ja-JP':'簡体字中国語'},
  'English': {'zh-CN':'English','en-US':'English','ja-JP':'英語'},
  '日本語': {'zh-CN':'日本語','en-US':'Japanese','ja-JP':'日本語'},
  '已选': {'zh-CN':'已选','en-US':'Selected','ja-JP':'選択中'},
  '玻璃等级': {'zh-CN':'玻璃等级','en-US':'Glass level','ja-JP':'ガラスレベル'},
  '级别越高越「玻璃」，开销也越大': {'zh-CN':'级别越高越「玻璃」，开销也越大','en-US':'Higher levels use stronger glass effects and more resources','ja-JP':'レベルが高いほどガラス効果が強くなり、負荷も増えます'},
}

export function getUiLanguage(): UiLanguage {
  const value = getConfig().language
  return value === 'en-US' || value === 'ja-JP' ? value : 'zh-CN'
}

export function setUiLanguage(language: UiLanguage) {
  saveConfig({language})
}

export function translate(text: string, language = getUiLanguage()): string {
  return DICT[text]?.[language] ?? text
}

const subscribe = (cb: () => void) => onConfigChange(cb)
const getSnapshot = () => getUiLanguage()

export function useI18n() {
  const language = useSyncExternalStore(subscribe, getSnapshot, getSnapshot)
  const t = (text: string) => translate(text, language)
  return {language, t, setLanguage: setUiLanguage}
}
