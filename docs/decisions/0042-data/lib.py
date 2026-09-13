import json, math, os
def hx(s): s=s.lstrip('#'); return tuple(int(s[i:i+2],16) for i in (0,2,4))
def h(c): return '#%02x%02x%02x'%c
def rround(x): return math.floor(x+0.5)  # Rust f64::round for non-negative
def mix(a,b,t): return tuple(max(0,min(255,rround(a[i]+(b[i]-a[i])*t))) for i in range(3))
def lin(v):
    v=v/255; return v/12.92 if v<=0.04045 else ((v+0.055)/1.055)**2.4
def lum(c):
    r,g,b=[lin(x) for x in c]; return 0.2126*r+0.7152*g+0.0722*b
def cr(a,b):
    la,lb=sorted([lum(a),lum(b)],reverse=True); return (la+0.05)/(lb+0.05)
def oklab(c):
    r,g,b=[lin(x) for x in c]
    l=0.4122214708*r+0.5363325363*g+0.0514459929*b
    m=0.2119034982*r+0.6806995451*g+0.1073969566*b
    s=0.0883024619*r+0.2817188376*g+0.6299787005*b
    l,m,s=[math.copysign(abs(x)**(1/3),x) for x in (l,m,s)]
    return (0.2104542553*l+0.7936177850*m-0.0040720468*s,
            1.9779984951*l-2.4285922050*m+0.4505937099*s,
            0.0259040371*l+0.7827717662*m-0.8086757660*s)
def oklch(c):
    L,a,b=oklab(c); C=math.hypot(a,b); H=math.degrees(math.atan2(b,a))%360
    return L,C,H
THEMES=json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'omarchy-themes.json')))
