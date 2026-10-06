import {GlassSwitch} from '@ttqtt/liquid-glass-react'
import './SwitchRow.css'

/**
 * 一行开关：左边标签 + 说明，右边库的 `GlassSwitch`。
 *
 * ⚠️ 库的 switch **禁用后不响应点击**，也没有原生 title 可读，所以禁用的理由必须走
 * `disabledHint` 拼进说明里 —— 不然用户只会以为开关坏了。
 */
export function SwitchRow({
                              label,
                              desc,
                              checked,
                              onChange,
                              disabled,
                              disabledHint,
                          }: {
    label: string
    desc?: string
    checked: boolean
    onChange: (v: boolean) => void
    disabled?: boolean
    disabledHint?: string
}) {
    const text = disabled && disabledHint && desc ? `${desc}（${disabledHint}）` : desc
    return (
        <div className="switch-row">
            <div className="switch-text">
                <span className="switch-label">{label}</span>
                {text ? <span className="switch-desc">{text}</span> : null}
            </div>
            <GlassSwitch aria-label={label} checked={checked} disabled={disabled} onCheckedChange={onChange}/>
        </div>
    )
}
