// GPS 坐标转换：WGS-84（EXIF 原始坐标）→ GCJ-02（国测局火星坐标）→ BD-09（百度坐标）。
// 百度地图使用 BD-09，国内坐标不经转换直接传给百度会偏移数百米（GCJ-02 加密偏移 +
// 百度二次偏移）；中国境外无加密偏移，坐标直通。
// 算法为 eviltransform / coordtransform 同源标准实现，参考值已用 npm coordtransform
// 2.1.2 官方输出逐位核对（见 geo.test.ts）。

const PI = Math.PI
/** 半长轴（克拉索夫斯基椭球，米） */
const A = 6378245.0
/** 偏心率平方 */
const EE = 0.00669342162296594323
/** 百度偏移角常量（π×3000/180，注意不是 π×3000） */
const X_PI = (PI * 3000.0) / 180.0

/** 是否在中国境外（境外无 GCJ-02 偏移，直通） */
export function outOfChina(lat: number, lng: number): boolean {
  return lng < 72.004 || lng > 137.8347 || lat < 0.8293 || lat > 55.8271
}

/** GCJ-02 纬度偏移量（WGS-84 参考椭球上的非线性加密函数） */
function transformLat(x: number, y: number): number {
  let ret = -100.0 + 2.0 * x + 3.0 * y + 0.2 * y * y + 0.1 * x * y + 0.2 * Math.sqrt(Math.abs(x))
  ret += ((20.0 * Math.sin(6.0 * x * PI) + 20.0 * Math.sin(2.0 * x * PI)) * 2.0) / 3.0
  ret += ((20.0 * Math.sin(y * PI) + 40.0 * Math.sin((y / 3.0) * PI)) * 2.0) / 3.0
  ret += ((160.0 * Math.sin((y / 12.0) * PI) + 320 * Math.sin((y * PI) / 30.0)) * 2.0) / 3.0
  return ret
}

/** GCJ-02 经度偏移量（同上） */
function transformLon(x: number, y: number): number {
  let ret = 300.0 + x + 2.0 * y + 0.1 * x * x + 0.1 * x * y + 0.1 * Math.sqrt(Math.abs(x))
  ret += ((20.0 * Math.sin(6.0 * x * PI) + 20.0 * Math.sin(2.0 * x * PI)) * 2.0) / 3.0
  ret += ((20.0 * Math.sin(x * PI) + 40.0 * Math.sin((x / 3.0) * PI)) * 2.0) / 3.0
  ret += ((150.0 * Math.sin((x / 12.0) * PI) + 300.0 * Math.sin((x / 30.0) * PI)) * 2.0) / 3.0
  return ret
}

/** WGS-84 → GCJ-02（境外直通，返回 [lat, lng]） */
function wgs84ToGcj02(lat: number, lng: number): [number, number] {
  if (outOfChina(lat, lng)) return [lat, lng]
  let dLat = transformLat(lng - 105.0, lat - 35.0)
  let dLon = transformLon(lng - 105.0, lat - 35.0)
  const radLat = (lat / 180.0) * PI
  let magic = Math.sin(radLat)
  magic = 1 - EE * magic * magic
  const sqrtMagic = Math.sqrt(magic)
  dLat = (dLat * 180.0) / (((A * (1 - EE)) / (magic * sqrtMagic)) * PI)
  dLon = (dLon * 180.0) / ((A / sqrtMagic) * Math.cos(radLat) * PI)
  return [lat + dLat, lng + dLon]
}

/** GCJ-02 → BD-09 */
function gcj02ToBd09(lat: number, lng: number): [number, number] {
  const z = Math.sqrt(lng * lng + lat * lat) + 0.00002 * Math.sin(lat * X_PI)
  const theta = Math.atan2(lat, lng) + 0.000003 * Math.cos(lng * X_PI)
  return [z * Math.sin(theta) + 0.006, z * Math.cos(theta) + 0.0065]
}

/**
 * WGS-84（EXIF GPS）→ BD-09（百度地图），返回 { lat, lng }。
 * 中国境外坐标直通（无加密偏移）。精度：算法与官方 coordtransform 逐位一致，
 * 国内偏移约数百米级修正。
 */
export function wgs84ToBd09(lat: number, lng: number): { lat: number; lng: number } {
  // 境外无 GCJ-02/BD-09 加密偏移，直接短路返回（否则 gcj02ToBd09 全局公式仍会错误偏移）
  if (outOfChina(lat, lng)) return { lat, lng }
  const [gLat, gLng] = wgs84ToGcj02(lat, lng)
  const [bLat, bLng] = gcj02ToBd09(gLat, gLng)
  return { lat: bLat, lng: bLng }
}
