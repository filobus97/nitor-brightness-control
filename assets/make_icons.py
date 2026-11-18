# Generates assets/nitor.ico and assets/nitor-plus.ico (run from assets/: python3 make_icons.py).
# Large sizes: white "N" whose right stroke glows in the edition colour; 16-24 px use a
# simplified, solid-stroke variant so the letter stays crisp. Needs Pillow.
from PIL import Image, ImageDraw
NAVY=(0x1C,0x27,0x36); WHITE=(255,255,255)
BLUE=(0x37,0x8A,0xDD); AMBER=(0xF2,0xA7,0x2B)
S=1024

def make(accent, small=False):
    im=Image.new("RGBA",(S,S),(0,0,0,0)); d=ImageDraw.Draw(im)
    m=24 if small else 40
    d.rounded_rectangle([m,m,S-m,S-m],radius=200 if small else 220,fill=NAVY)
    # letter geometry (thicker when small)
    sw=150 if small else 118           # stem width
    top,bot=236,788
    lx=262; rx=S-262-sw
    # left stem
    d.rectangle([lx,top,lx+sw,bot],fill=WHITE)
    # diagonal: from top of left stem to bottom of right stem
    d.polygon([(lx,top),(lx+sw*1.15,top),(rx+sw,bot),(rx+sw-sw*1.15,bot)],fill=WHITE)
    # right stem: accent, fading upward (glow) at large sizes, solid when small
    if small:
        d.rectangle([rx,top,rx+sw,bot],fill=accent)
    else:
        from PIL import ImageFilter
        h=bot-top
        # soft glow around the stem, strongest at the bottom
        glow=Image.new("RGBA",(S,S),(0,0,0,0)); g=ImageDraw.Draw(glow)
        for i in range(h):
            t=i/h
            g.line([(rx-24,top+i),(rx+sw+24,top+i)],fill=accent+(int(110*t*t),))
        glow=glow.filter(ImageFilter.GaussianBlur(30))
        im=Image.alpha_composite(im,glow); d=ImageDraw.Draw(im)
        # opaque stem: colour blends from navy-ish (top) to full accent (bottom)
        for i in range(h):
            t=i/h; k=0.30+0.70*t
            col=tuple(int(NAVY[c]+(accent[c]-NAVY[c])*k) for c in range(3))
            d.line([(rx,top+i),(rx+sw,top+i)],fill=col+(255,))
        d.polygon([(lx,top),(lx+sw*1.15,top),(rx+sw,bot),(rx+sw-sw*1.15,bot)],fill=WHITE)
    return im

def ico(accent, path):
    big=make(accent); small=make(accent, small=True)
    imgs=[]
    for n in (256,64,48,40,32,24,20,16):
        src=small if n<=24 else big
        imgs.append(src.resize((n,n),Image.LANCZOS))
    imgs[0].save(path,format="ICO",sizes=[(i.width,i.height) for i in imgs],append_images=imgs[1:])
    return big, small

pb, ps = ico(BLUE, "nitor.ico")
qb, qs = ico(AMBER, "nitor-plus.ico")
