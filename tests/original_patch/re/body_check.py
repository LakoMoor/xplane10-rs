"""body_check.py <port.ppm> <oracle.png> l t r b — compare a translucent menu body with the oracle's
missing background compensated (frame alpha 236/255 over the oracle's own sky at x=bgx)."""
import sys; import os; sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from png import read_png
d=open(sys.argv[1],'rb').read(); a=d.split(b'\n',3); w=int(a[1].split()[0]); P=a[3]
W,H,B,O=read_png(sys.argv[2]); l,t,r,b=map(int,sys.argv[3:7]); bgx=int(sys.argv[7]) if len(sys.argv)>7 else 1000
n=big=0; tot=0; cols={}
for y in range(t,b):
    for x in range(l,r):
        for k in range(3):
            bg=O[(y*W+bgx)*B+k]; p=P[(y*w+x)*3+k]+(1-236/255)*bg; o=O[(y*W+x)*B+k]
            e=abs(p-o); n+=1; tot+=e
            if e>12: big+=1; cols.setdefault(y,set()).add(x)
print(f"body {l},{t}-{r},{b}: mean {tot/n:.2f}, samples off >12: {big} of {n}")
for y in sorted(cols)[:12]: print(" ", y, sorted(cols[y])[:16])
