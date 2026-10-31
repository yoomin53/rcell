# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 도형 윤곽 따라 그리기 (1/3): 한셀 화면 갈무리(screen-capture.ps1)에서 도형 하나를 칠 종류별로 벡터화한다.
# 도형은 make-shapes-big.ps1 이 만든 큰 모양 (240×240pt)이고, 갈무리에서 타일 하나(영역)를 잘라 윤곽을 낸다.
#   -Tiles "x,y,w,h,이름;..." 이면 $Out\이름.txt 로 모두 쓴다. 2/3 = shape-trace-gen.js (Rust 표 만들기)
param([string]$Png, [int]$X, [int]$Y, [int]$W, [int]$H, [string]$Out, [double]$Eps = 1.0, [int]$MinArea = 30, [string]$Tiles = "")
# vectorize one shape in a HCell capture. Output per fill class: rings (px, region-relative, corner coordinates), `S` when the class is stroked
# (its mask is then c + stroke pixels, pulled in by half a pixel to the stroke centre line)
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Globalization;
using System.Runtime.InteropServices;
using System.Text;
public static class ST {
    static readonly int[][] Pal = new int[][] {
        new int[]{0x61,0x82,0xD6}, // 0 norm
        new int[]{0x49,0x62,0xA1}, // 1 darkenLess
        new int[]{0x31,0x41,0x6B}, // 2 darken
        new int[]{0x90,0xA7,0xE2}, // 3 lighten
        new int[]{0x79,0x95,0xDC}, // 4 lightenLess
        new int[]{0x2E,0x3E,0x67}, // 5 stroke
    };
    static readonly string[] Names = { "norm", "darkenLess", "darken", "lighten", "lightenLess", "stroke" };
    public static string Run(string path, int x0, int y0, int w, int h, double eps, int minArea) {
        byte[] buf; int stride;
        using (var b = new Bitmap(path)) {
            var d = b.LockBits(new Rectangle(0, 0, b.Width, b.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            stride = d.Stride; buf = new byte[stride * b.Height]; Marshal.Copy(d.Scan0, buf, 0, buf.Length); b.UnlockBits(d);
        }
        var cls = new int[w, h];
        var sm = new int[w, h];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int o = (y0 + y) * stride + (x0 + x) * 4; int B = buf[o], G = buf[o + 1], R = buf[o + 2];
            sm[x, y] = R + G + B;
            int best = -1, bd = 1000;
            for (int c = 0; c < Pal.Length; c++) {
                int dd = Math.Max(Math.Abs(R - Pal[c][0]), Math.Max(Math.Abs(G - Pal[c][1]), Math.Abs(B - Pal[c][2])));
                if (dd < bd) { bd = dd; best = c; }
            }
            cls[x, y] = bd <= 26 ? best : -1;
        }
        var sb = new StringBuilder();
        for (int c = 0; c < Pal.Length; c++) {
            var mask = new bool[w, h];
            int n = 0, nb = 0, nbs = 0;
            for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) if (cls[x, y] == c) { mask[x, y] = true; n++; }
            if (c == 0) FillHoles(mask, cls, w, h);
            bool stroked = false;
            if (c != 5 && n > 0) {
                // boundary pixels of the class next to stroke pixels
                for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) if (mask[x, y]) {
                    bool edge = false, st = false;
                    for (int dx = -2; dx <= 2; dx++) for (int dy = -2; dy <= 2; dy++) {
                        if (dx == 0 && dy == 0) continue; int nx = x + dx, ny = y + dy;
                        if (nx < 0 || ny < 0 || nx >= w || ny >= h) continue;
                        // 선 = 선 색이거나, 선과 칠이 섞여 어느 칠도 아닌 어두운 화소 (흰 배경과 섞인 화소는 밝다). 2px 안까지 본다
                        // (선과 칠 사이에 섞인 화소가 끼어 있다)
                        if (cls[nx, ny] != c) {
                            if (Math.Abs(dx) <= 1 && Math.Abs(dy) <= 1) edge = true;
                            if (cls[nx, ny] == 5 || (cls[nx, ny] == -1 && sm[nx, ny] < 400)) st = true;
                        }
                    }
                    if (edge) { nb++; if (st) nbs++; }
                }
                stroked = nb > 0 && nbs * 2 > nb;
            }
            sb.Append(Names[c]).Append(stroked ? " S" : "").Append(' ').Append(n).Append(" nb=").Append(nb).Append(" st=").Append(nbs).Append('\n');
            var rings = Rings(mask, w, h, minArea);
            foreach (var ring in rings) {
                var simp = Simplify(ring, eps);
                if (c != 5) { double ar = 0; for (int i = 0; i < simp.Count; i++) { var p = simp[i]; var q = simp[(i + 1) % simp.Count]; ar += p[0] * q[1] - q[0] * p[1]; } if (Math.Abs(ar) / 2 < minArea) continue; }
                if (stroked) simp = Inset(simp, -0.5);
                if (c == 5) simp = Inset(simp, 0.5);
                sb.Append("R ");
                foreach (var p in simp) sb.Append(Math.Round(p[0], 2).ToString(CultureInfo.InvariantCulture)).Append(',').Append(Math.Round(p[1], 2).ToString(CultureInfo.InvariantCulture)).Append(' ');
                sb.Append('\n');
            }
        }
        return sb.ToString();
    }
    // 보통 칠의 구멍 가운데 다른 칠(또는 선)이 채운 것은 메운다 — 그 위에 덮는 칠과 틈이 생기지 않게. 배경이 보이는 구멍(흰색 화소가
    // 절반 넘는)은 남긴다
    static void FillHoles(bool[,] mask, int[,] cls, int w, int h) {
        var outside = new bool[w, h];
        var st = new Stack<int[]>();
        for (int x = 0; x < w; x++) { st.Push(new int[] { x, 0 }); st.Push(new int[] { x, h - 1 }); }
        for (int y = 0; y < h; y++) { st.Push(new int[] { 0, y }); st.Push(new int[] { w - 1, y }); }
        while (st.Count > 0) {
            var p = st.Pop(); int x = p[0], y = p[1];
            if (x < 0 || y < 0 || x >= w || y >= h || outside[x, y] || mask[x, y]) continue;
            outside[x, y] = true;
            st.Push(new int[] { x + 1, y }); st.Push(new int[] { x - 1, y }); st.Push(new int[] { x, y + 1 }); st.Push(new int[] { x, y - 1 });
        }
        var seen = new bool[w, h];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) if (!mask[x, y] && !outside[x, y] && !seen[x, y]) {
            var comp = new List<int[]>(); var s2 = new Stack<int[]>(); s2.Push(new int[] { x, y }); seen[x, y] = true;
            int other = 0;
            while (s2.Count > 0) {
                var p = s2.Pop(); comp.Add(p); if (cls[p[0], p[1]] >= 0) other++;
                int[][] nb = { new int[] { 1, 0 }, new int[] { -1, 0 }, new int[] { 0, 1 }, new int[] { 0, -1 } };
                foreach (var d in nb) {
                    int nx = p[0] + d[0], ny = p[1] + d[1];
                    if (nx >= 0 && ny >= 0 && nx < w && ny < h && !mask[nx, ny] && !outside[nx, ny] && !seen[nx, ny]) { seen[nx, ny] = true; s2.Push(new int[] { nx, ny }); }
                }
            }
            if (other * 2 > comp.Count) foreach (var p in comp) mask[p[0], p[1]] = true;
        }
    }
    // move every edge 0.5 to the right (into the mask), miter joins
    static List<double[]> Inset(List<double[]> r, double d) {
        int n = r.Count; var res = new List<double[]>();
        if (n < 3) return r;
        for (int i = 0; i < n; i++) {
            var a = r[(i + n - 1) % n]; var b = r[i]; var c = r[(i + 1) % n];
            double e1x = b[0] - a[0], e1y = b[1] - a[1], e2x = c[0] - b[0], e2y = c[1] - b[1];
            double l1 = Math.Sqrt(e1x * e1x + e1y * e1y), l2 = Math.Sqrt(e2x * e2x + e2y * e2y);
            if (l1 < 1e-9 || l2 < 1e-9) { res.Add(b); continue; }
            // right-hand normal in y-down coordinates of direction (dx,dy) is (-dy, dx)
            double n1x = -e1y / l1, n1y = e1x / l1, n2x = -e2y / l2, n2y = e2x / l2;
            double bx = n1x + n2x, by = n1y + n2y; double bl = bx * bx + by * by;
            if (bl < 1e-6) { res.Add(new double[] { b[0] + n1x * d, b[1] + n1y * d }); continue; }
            double k = 2.0 * d / bl; if (k * Math.Sqrt(bl) > 3 * d) k = 3 * d / Math.Sqrt(bl);
            res.Add(new double[] { b[0] + bx * k, b[1] + by * k });
        }
        return res;
    }
    static List<List<int[]>> Rings(bool[,] mask, int w, int h, int minArea) {
        var seen = new bool[w, h];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) if (mask[x, y] && !seen[x, y]) {
            var st = new Stack<int[]>(); var comp = new List<int[]>(); st.Push(new int[] { x, y }); seen[x, y] = true;
            while (st.Count > 0) { var p = st.Pop(); comp.Add(p);
                for (int dx = -1; dx <= 1; dx++) for (int dy = -1; dy <= 1; dy++) {
                    int nx = p[0] + dx, ny = p[1] + dy;
                    if (nx >= 0 && ny >= 0 && nx < w && ny < h && mask[nx, ny] && !seen[nx, ny]) { seen[nx, ny] = true; st.Push(new int[] { nx, ny }); } } }
            if (comp.Count < minArea) foreach (var p in comp) mask[p[0], p[1]] = false;
        }
        Func<int, int, bool> M = (x, y) => x >= 0 && y >= 0 && x < w && y < h && mask[x, y];
        var edges = new Dictionary<long, List<int[]>>();
        Action<int, int, int, int> add = (ax, ay, bx, by) => {
            long k = (long)ax * 1048576L + (long)ay; List<int[]> l;
            if (!edges.TryGetValue(k, out l)) { l = new List<int[]>(); edges[k] = l; }
            l.Add(new int[] { bx, by });
        };
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) if (mask[x, y]) {
            if (!M(x, y - 1)) add(x, y, x + 1, y);
            if (!M(x + 1, y)) add(x + 1, y, x + 1, y + 1);
            if (!M(x, y + 1)) add(x + 1, y + 1, x, y + 1);
            if (!M(x - 1, y)) add(x, y + 1, x, y);
        }
        var rings = new List<List<int[]>>();
        foreach (var kv in new List<KeyValuePair<long, List<int[]>>>(edges)) {
            while (kv.Value.Count > 0) {
                int sx = (int)(kv.Key / 1048576L), sy = (int)(kv.Key % 1048576L);
                var ring = new List<int[]>(); int cx = sx, cy = sy;
                var e = kv.Value[0]; kv.Value.RemoveAt(0);
                int pdx = e[0] - cx, pdy = e[1] - cy; ring.Add(new int[] { cx, cy }); cx = e[0]; cy = e[1];
                int guard = 0;
                while (!(cx == sx && cy == sy) && guard++ < 400000) {
                    ring.Add(new int[] { cx, cy });
                    long k = (long)cx * 1048576L + (long)cy; List<int[]> l;
                    if (!edges.TryGetValue(k, out l) || l.Count == 0) break;
                    int pick = 0;
                    if (l.Count > 1) {
                        int bestScore = -9;
                        for (int i = 0; i < l.Count; i++) {
                            int ndx = l[i][0] - cx, ndy = l[i][1] - cy;
                            int cross = pdx * ndy - pdy * ndx;
                            if (cross > bestScore) { bestScore = cross; pick = i; }
                        }
                    }
                    var ne = l[pick]; l.RemoveAt(pick);
                    pdx = ne[0] - cx; pdy = ne[1] - cy; cx = ne[0]; cy = ne[1];
                }
                if (ring.Count >= 4) rings.Add(ring);
            }
        }
        return rings;
    }
    static List<double[]> Simplify(List<int[]> ring, double eps) {
        int n = ring.Count;
        int far = 0; double fd = -1;
        for (int i = 1; i < n; i++) { double dx = ring[i][0] - ring[0][0], dy = ring[i][1] - ring[0][1]; double dd = dx * dx + dy * dy; if (dd > fd) { fd = dd; far = i; } }
        var pts = new List<double[]>(); foreach (var p in ring) pts.Add(new double[] { p[0], p[1] });
        var keep = new bool[n]; keep[0] = true; keep[far] = true;
        DP(pts, 0, far, eps, keep);
        var tail = new List<double[]>(); for (int i = far; i < n; i++) tail.Add(pts[i]); tail.Add(pts[0]);
        var keep2 = new bool[tail.Count]; keep2[0] = true; keep2[tail.Count - 1] = true; DP(tail, 0, tail.Count - 1, eps, keep2);
        var res = new List<double[]>();
        for (int i = 0; i <= far; i++) if (keep[i]) res.Add(pts[i]);
        for (int i = 1; i < tail.Count - 1; i++) if (keep2[i]) res.Add(tail[i]);
        return res;
    }
    static void DP(List<double[]> p, int a, int b, double eps, bool[] keep) {
        if (b <= a + 1) return;
        double ax = p[a][0], ay = p[a][1], bx = p[b][0], by = p[b][1];
        double dx = bx - ax, dy = by - ay, len = Math.Sqrt(dx * dx + dy * dy);
        int idx = -1; double md = -1;
        for (int i = a + 1; i < b; i++) {
            double d = len < 1e-9 ? Math.Sqrt((p[i][0] - ax) * (p[i][0] - ax) + (p[i][1] - ay) * (p[i][1] - ay))
                                  : Math.Abs(dy * (p[i][0] - ax) - dx * (p[i][1] - ay)) / len;
            if (d > md) { md = d; idx = i; }
        }
        if (md > eps) { keep[idx] = true; DP(p, a, idx, eps, keep); DP(p, idx, b, eps, keep); }
    }
}
"@
if ($Tiles) {
    # "x,y,w,h,name;x,y,w,h,name" -> $Out\name.txt
    foreach ($t in ($Tiles -split ";")) {
        $f = $t -split ","
        [IO.File]::WriteAllText((Join-Path $Out ($f[4] + ".txt")), [ST]::Run($Png, [int]$f[0], [int]$f[1], [int]$f[2], [int]$f[3], $Eps, $MinArea), (New-Object Text.UTF8Encoding $false))
    }
} else {
    [IO.File]::WriteAllText($Out, [ST]::Run($Png, $X, $Y, $W, $H, $Eps, $MinArea), (New-Object Text.UTF8Encoding $false))
}
"ok"
