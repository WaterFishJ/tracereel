#pragma once
#include <QtGui/QPainter>
#include <QtGui/QPainterPath>
#include <QtGui/QFontMetricsF>
#include <QtGui/QLinearGradient>
#include <QtGui/QRadialGradient>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonObject>
#include <QtCore/QCache>
#include <algorithm>
#include <cmath>
#include <vector>

// All geometry is in the 1080-high design space, also used by the QML editor.
static QString ovHudLabel(const QString &metric) {
    if(metric=="speed") return QStringLiteral("速度");
    if(metric=="heading") return QStringLiteral("航向");
    if(metric=="heart_rate") return QStringLiteral("心率");
    if(metric=="cadence") return QStringLiteral("步频");
    if(metric=="elevation") return QStringLiteral("海拔");
    if(metric=="elapsed") return QStringLiteral("总用时");
    if(metric=="segment") return QStringLiteral("分段用时");
    if(metric=="pace") return QStringLiteral("配速");
    if(metric=="distance") return QStringLiteral("距离");
    return metric;
}
static QString ovHudValue(const QString &metric,const QJsonObject &v) {
    auto x=v.value(metric);
    if(x.isNull() || x.isUndefined()) return "--";
    if(x.isString()) return x.toString();
    QString unit;
    int dp=0;
    if(metric=="speed") {unit=" km/h";dp=1;}
    if(metric=="distance") {unit=" km";dp=2;}
    if(metric=="heart_rate") unit=" bpm";
    if(metric=="cadence") unit=QStringLiteral(" 步/分");
    if(metric=="heading") unit=QStringLiteral("°");
    if(metric=="elevation") unit=" m";
    return QString::number(x.toDouble(),'f',dp)+unit;
}
static void ovHudText(QPainter *p,QRectF r,QString text,QColor color,double size,int align=Qt::AlignCenter) {
    p->setPen(color);
    QFont f(QStringLiteral("Microsoft YaHei"));
    f.setPixelSize(std::max(8,int(size))); p->setFont(f);
    QFontMetricsF fm(f);
    double tw=fm.horizontalAdvance(text);
    if(tw>r.width() && tw>0) { f.setPixelSize(std::max(8,int(size*r.width()/tw))); p->setFont(f); }
    p->drawText(r,align,text);
}
static QPointF ovPolar(double cx,double cy,double r,double angle) {
    double a=angle*3.141592653589793/180.0;
    return QPointF(cx+r*std::cos(a),cy+r*std::sin(a));
}
static void ovDrawHud(QPainter *p,const QString &layout,const QString &data,double width,double height,const double *points,int count) {
    if(width<=0 || height<=0) return;
    struct LayoutCache {QString raw;std::vector<QJsonValue> widgets;};
    static thread_local LayoutCache lc;
    static thread_local QCache<QString,QImage> backgrounds(32);
    if(lc.raw!=layout) {
        lc.raw=layout; lc.widgets.clear();
        for(auto w:QJsonDocument::fromJson(layout.toUtf8()).object()["widgets"].toArray()) lc.widgets.push_back(w);
        std::stable_sort(lc.widgets.begin(),lc.widgets.end(),[](const QJsonValue &a,const QJsonValue &b){return a.toObject()["z"].toInt()<b.toObject()["z"].toInt();});
    }
    auto v=QJsonDocument::fromJson(data.toUtf8()).object();
    const double scale=height/1080.0, canvasW=width/scale;
    p->save();p->scale(scale,scale);
    for(auto entry:lc.widgets) {
        auto w=entry.toObject();
        if(!w["visible"].toBool(true)) continue;
        QString metric=w["metric"].toString(),variant=w["variant"].toString();
        double ww=w["w"].toDouble()*canvasW,hh=w["h"].toDouble()*1080;
        if(ww<=0 || hh<=0) continue;
        QColor accent(w["accent"].toString()),bg(w["background"].toString()),fg(w["foreground"].toString());
        if(!accent.isValid())accent=QColor("#00e5a0");
        if(!bg.isValid())bg=QColor("#17202b");
        if(!fg.isValid())fg=Qt::white;
        double fs=w["font_size"].toDouble(28);
        p->save();p->setOpacity(w["opacity"].toDouble(0.9));
        p->translate(w["x"].toDouble()*canvasW+ww/2,w["y"].toDouble()*1080+hh/2);
        p->rotate(w["rotation"].toDouble());p->translate(-ww/2,-hh/2);
        QRectF box(0,0,ww,hh);
        p->setRenderHint(QPainter::Antialiasing);
        p->setRenderHint(QPainter::TextAntialiasing);
        QColor muted=fg;muted.setAlphaF(.60);
        QColor line=fg;line.setAlphaF(.13);
        if(variant!="compass" && variant!="arc" && variant!="dial") {
            QLinearGradient surface(0,0,ww,hh);surface.setColorAt(0,bg.lighter(125));surface.setColorAt(1,bg);
            p->setPen(Qt::NoPen);p->setBrush(surface);p->drawRoundedRect(box.adjusted(1,1,-1,-1),3,3);
            p->setPen(QPen(accent,2));p->drawLine(QPointF(1,1),QPointF(ww-1,1));
        }
        if(metric=="classic") {
            QStringList keys={"elapsed","segment","speed","pace","heart_rate","distance","heading","elevation","cadence"};
            auto flags=v["classic_flags"].toArray();
            const bool hasFields=w.contains("fields") && w["fields"].isArray();
            QStringList fields;for(auto f:w["fields"].toArray())fields.push_back(f.toString());
            auto enabled=[&](int i){return hasFields?fields.contains(keys[i]):flags[i].toBool();};
            int rows=0;for(int i=0;i<keys.size();++i)if(enabled(i))++rows;
            double pad=std::min(18.0,ww*.06),rh=(hh-2*pad)/std::max(1,rows),y=pad;
            double labelWidth=ww*.42,valueX=pad+labelWidth+8;
            for(int i=0;i<keys.size();++i) if(enabled(i)) {
                ovHudText(p,QRectF(pad,y,labelWidth,rh),ovHudLabel(keys[i]),muted,std::min(fs,rh*.55),Qt::AlignLeft|Qt::AlignVCenter);
                ovHudText(p,QRectF(valueX,y,ww-pad-valueX,rh),ovHudValue(keys[i],v),fg,std::min(fs*1.1,rh*.60),Qt::AlignRight|Qt::AlignVCenter);
                y+=rh;
                if(y<hh-pad-1){p->setPen(QPen(line,1));p->drawLine(QPointF(pad,y),QPointF(ww-pad,y));}
            }
        } else if(variant=="arc" || variant=="dial") {
            double max=w["automatic"].toBool(true)?v["max_speed"].toDouble(30):w["maximum"].toDouble(30);
            max=std::max(1.0,max);
            double r=std::min(ww*.40,hh*.36),cx=ww/2,cy=hh*.49;
            double start=variant=="arc"?180:135,sweep=variant=="arc"?180:270;
            QString key=QString::number(scale)+":"+QString::number(ww)+":"+QString::number(hh)+":"+variant+":"+accent.name()+":"+fg.name()+":"+QString::number(max)+":"+QString::number(fs);
            QImage *image=backgrounds.object(key);
            if(!image) {
                image=new QImage(int(std::ceil(ww*scale)),int(std::ceil(hh*scale)),QImage::Format_ARGB32_Premultiplied);image->setDevicePixelRatio(scale);image->fill(Qt::transparent);
                QPainter q(image);q.setRenderHint(QPainter::Antialiasing);
                q.setPen(QPen(accent,4));q.drawArc(QRectF(cx-r,cy-r,2*r,2*r),int(-start*16),int(-sweep*16));
                for(int i=0;i<=10;++i) {
                    double a=start+sweep*i/10;
                    q.setPen(QPen(fg,i%2?1.5:3));q.drawLine(ovPolar(cx,cy,r*.90,a),ovPolar(cx,cy,r,a));
                    if(i%2==0) {auto pt=ovPolar(cx,cy,r*.70,a);ovHudText(&q,QRectF(pt.x()-25,pt.y()-12,50,24),QString::number(max*i/10,'f',0),fg,fs*.52);}
                }
                q.end();backgrounds.insert(key,image);
            }
            p->drawImage(QPointF(0,0),*image);
            if(!v[metric].isNull()) {
                double a=start+sweep*std::clamp(v[metric].toDouble()/max,0.0,1.0);
                p->setPen(QPen(accent,variant=="arc"?9:4,Qt::SolidLine,Qt::RoundCap));
                if(variant=="arc") {
                    p->drawArc(QRectF(cx-r,cy-r,2*r,2*r),int(-start*16),int(-(a-start)*16));
                    p->setPen(Qt::NoPen);p->setBrush(fg);p->drawEllipse(ovPolar(cx,cy,r,a),4,4);
                } else {
                    p->drawLine(QPointF(cx,cy),ovPolar(cx,cy,r*.86,a));
                    p->setBrush(bg);p->drawEllipse(QPointF(cx,cy),8,8);
                }
            }
            ovHudText(p,QRectF(8,hh*.70,ww-16,hh*.17),ovHudValue(metric,v),fg,fs);
            ovHudText(p,QRectF(8,hh*.88,ww-16,hh*.10),ovHudLabel(metric),accent,fs*.55);
        } else if(variant=="compass" || variant=="ribbon") {
            bool valid=v["heading"].isDouble();
            double heading=v["heading"].toDouble();
            if(variant=="compass") {
                double r=std::min(ww*.44,hh*.34),cx=ww/2,cy=hh*.41;
                p->setPen(QPen(accent,1.5));p->setBrush(bg);p->drawEllipse(QPointF(cx,cy),r,r);
                p->setPen(QPen(line,1));p->setBrush(Qt::NoBrush);p->drawEllipse(QPointF(cx,cy),r*.94,r*.94);
                // The compass card rotates beneath a fixed forward marker.
                p->save();p->translate(cx,cy);p->rotate(valid?-heading:0);
                for(int i=0;i<72;++i) {
                    double a=-90+i*5;bool major=i%6==0;
                    p->setPen(QPen(i==0?QColor("#ff6577"):major?fg:muted,major?2:1));
                    p->drawLine(ovPolar(0,0,r*(major?.80:.87),a),ovPolar(0,0,r*.93,a));
                }
                QStringList dirs={"N","E","S","W"};
                for(int i=0;i<4;++i) {
                    auto pt=ovPolar(0,0,r*.66,-90+i*90);
                    p->save();p->translate(pt);p->rotate(valid?heading:0);
                    ovHudText(p,QRectF(-16,-16,32,32),dirs[i],i==0?QColor("#ff6577"):fg,fs*.57);p->restore();
                }
                if(valid) {
                    QPolygonF north,south;north<<QPointF(0,-r*.43)<<QPointF(-r*.11,0)<<QPointF(r*.11,0);
                    south<<QPointF(0,r*.43)<<QPointF(-r*.11,0)<<QPointF(r*.11,0);
                    p->setPen(Qt::NoPen);p->setBrush(QColor("#ff6577"));p->drawPolygon(north);p->setBrush(muted);p->drawPolygon(south);
                }
                p->restore();
                p->setBrush(accent);p->setPen(Qt::NoPen);QPolygonF marker;
                marker<<QPointF(cx,cy-r*.81)<<QPointF(cx-6,cy-r-5)<<QPointF(cx+6,cy-r-5);p->drawPolygon(marker);
                p->setBrush(fg);p->drawEllipse(QPointF(cx,cy),3,3);
            } else {
                p->save();p->setClipRect(QRectF(8,8,ww-16,hh*.63));
                for(int i=-8;i<=8;++i) {
                    double deg=std::floor(heading/15)*15+i*15,x=ww/2+(deg-heading)*ww/110;
                    if(!valid)continue;
                    p->setPen(QPen(fg,2));p->drawLine(QPointF(x,hh*.40),QPointF(x,hh*.52));
                    int d=(int(deg)%360+360)%360;
                    QString label=d%90==0?QStringList({"N","E","S","W"})[d/90]:QString::number(d);
                    ovHudText(p,QRectF(x-25,10,50,hh*.26),label,fg,fs*.55);
                }
                p->restore();p->setPen(QPen(accent,3));p->drawLine(QPointF(ww/2,hh*.42),QPointF(ww/2,hh*.64));
            }
            ovHudText(p,QRectF(8,hh*.76,ww-16,hh*.2),ovHudValue(metric,v),accent,fs*.85);
        } else if(variant=="heart") {
            double phase=v["phase"].toDouble(),beat=0;
            if(!v["heart_rate"].isNull())beat=std::exp(-std::pow((phase-.12)/.10,2))+.45*std::exp(-std::pow((phase-.34)/.10,2));
            double size=std::min(ww,hh)*.36*(1+.17*beat);
            QColor heartColor=v["heart_rate"].isDouble()?accent:muted;
            QColor glow=heartColor;glow.setAlphaF(.06+.12*beat);
            QRadialGradient halo(QPointF(ww/2,hh*.34),size*1.15);halo.setColorAt(0,glow);halo.setColorAt(1,Qt::transparent);
            p->setPen(Qt::NoPen);p->setBrush(halo);p->drawEllipse(QPointF(ww/2,hh*.34),size*1.15,size*1.15);
            p->save();p->translate(ww/2,hh*.34);p->scale(size,size);
            QPainterPath heart;heart.moveTo(0,.45);heart.cubicTo(-1,-.12,-.5,-.85,0,-.35);heart.cubicTo(.5,-.85,1,-.12,0,.45);
            p->setPen(Qt::NoPen);p->setBrush(heartColor);p->drawPath(heart);p->restore();
            // A beat-synchronised trace, driven by integrated BPM rather than wall time.
            p->save();p->setClipRect(QRectF(12,hh*.50,ww-24,hh*.17));
            QPainterPath trace;
            for(int i=0;i<=100;++i) {
                double u=i/100.0,q=u-phase+.5;q-=std::floor(q);
                double pulse=std::exp(-std::pow((q-.50)/.022,2))-.40*std::exp(-std::pow((q-.46)/.025,2))-.32*std::exp(-std::pow((q-.54)/.03,2));
                QPointF pt(12+u*(ww-24),hh*.59-(v["heart_rate"].isDouble()?pulse*hh*.065:0));
                if(i==0)trace.moveTo(pt);else trace.lineTo(pt);
            }
            p->setPen(QPen(heartColor,1.5));p->setBrush(Qt::NoBrush);p->drawPath(trace);p->restore();
            ovHudText(p,QRectF(8,hh*.68,ww-16,hh*.20),ovHudValue(metric,v),fg,fs);
            ovHudText(p,QRectF(8,hh*.88,ww-16,hh*.10),ovHudLabel(metric),muted,fs*.45);
        } else if(variant=="profile" || variant=="scroll") {
            ovHudText(p,QRectF(12,4,ww-24,hh*.25),ovHudLabel(metric)+"  "+ovHudValue(metric,v),fg,fs*.75,Qt::AlignLeft|Qt::AlignVCenter);
            QRectF plot(12,hh*.32,ww-24,hh*.59);
            double now=v["track_time"].toDouble(),lo=0,hi=1,min=1e99,max=-1e99;
            if(count>0) {lo=points[0];hi=points[(count-1)*2];}
            if(variant=="scroll") {double win=w["window"].toDouble(300);lo=now-win/2;hi=now+win/2;}
            hi=std::max(lo+.001,hi);
            int first=0;while(first<count && points[first*2]<lo)++first;first=std::max(0,first-1);
            int last=first;while(last<count && points[last*2]<=hi)++last;last=std::min(count,last+1);
            for(int i=first;i<last;++i){min=std::min(min,points[i*2+1]);max=std::max(max,points[i*2+1]);}
            if(last>first) {
                if(max-min<1){min-=.5;max+=.5;}
                // Bounded cached geometry: full profile persists, scroll updates deterministically.
                static thread_local QString curveKey;
                static thread_local QPainterPath curve;
                QString ck=QString::number(quintptr(points))+":"+QString::number(count)+":"+QString::number(lo,'g',16)+":"+QString::number(hi,'g',16)+":"+QString::number(ww)+":"+QString::number(hh)+":"+QString::number(min,'g',16)+":"+QString::number(max,'g',16);
                if(curveKey!=ck) {curveKey=ck;curve=QPainterPath();for(int i=first;i<last;++i){QPointF pt(plot.left()+(points[i*2]-lo)/(hi-lo)*plot.width(),plot.bottom()-(points[i*2+1]-min)/(max-min)*plot.height());if(i==first)curve.moveTo(pt);else curve.lineTo(pt);}}
                p->save();p->setClipRect(plot);
                p->setPen(QPen(line,1));
                for(int i=1;i<4;++i){double y=plot.top()+plot.height()*i/4;p->drawLine(QPointF(plot.left(),y),QPointF(plot.right(),y));}
                QPainterPath area=curve;area.lineTo(curve.currentPosition().x(),plot.bottom());area.lineTo(plot.left(),plot.bottom());area.closeSubpath();
                QColor fill=accent;fill.setAlphaF(.28);QLinearGradient gradient(0,plot.top(),0,plot.bottom());gradient.setColorAt(0,fill);gradient.setColorAt(1,Qt::transparent);
                p->fillPath(area,gradient);p->setPen(QPen(accent,2.5));p->setBrush(Qt::NoBrush);p->drawPath(curve);
                if(!v["elevation"].isNull() && now>=lo && now<=hi) {
                    double x=plot.left()+(now-lo)/(hi-lo)*plot.width(),y=plot.bottom()-(v["elevation"].toDouble()-min)/(max-min)*plot.height();
                    p->setPen(QPen(fg,1));p->drawLine(QPointF(x,plot.top()),QPointF(x,plot.bottom()));p->setBrush(accent);p->drawEllipse(QPointF(x,y),4,4);
                }p->restore();
            }
        } else {
            // Icon geometry follows height; trimming the value column keeps it unchanged.
            double r=std::min(hh*.22,std::max(2.0,(ww-32)/4)),inset=2*r+24;
            {
                p->setPen(QPen(accent,2));p->setBrush(Qt::NoBrush);
                double x=12+r,y=hh*.5;
                if(metric=="pace") {
                    p->drawLine(QPointF(x-r,y+r),QPointF(x,y-r));QPolygonF flag;
                    flag<<QPointF(x,y-r)<<QPointF(x+r,y-r*.5)<<QPointF(x-r*.3,y);p->setBrush(QColor("#ff6577"));p->drawPolygon(flag);
                } else if(metric=="cadence") {
                    // Mirrored soles with an inward arch; left foot leads the right.
                    p->setPen(Qt::NoPen);p->setBrush(accent);
                    const double k=r*1.12;
                    for(int side=0;side<2;++side) {
                        p->save();
                        p->translate(x+(side?-1:1)*r*.47,y+(side?-1:1)*r*.34);
                        p->rotate(side?-10:10);
                        p->scale(side?-k:k,k);
                        // Rounded forefoot and heel, with no individual toe details.
                        QPainterPath sole;
                        sole.moveTo(-.03,-.64);
                        sole.cubicTo(-.23,-.64,-.31,-.49,-.30,-.27);
                        sole.cubicTo(-.30,-.09,-.12,.01,-.14,.20);
                        sole.cubicTo(-.15,.32,-.22,.40,-.20,.50);
                        sole.cubicTo(-.18,.67,.13,.69,.19,.51);
                        sole.cubicTo(.25,.34,.19,.17,.25,-.04);
                        sole.cubicTo(.31,-.24,.34,-.43,.22,-.56);
                        sole.cubicTo(.16,-.62,.06,-.65,-.03,-.64);
                        sole.closeSubpath();p->drawPath(sole);
                        p->restore();
                    }
                } else if(metric=="elapsed" || metric=="segment") {
                    p->drawEllipse(QPointF(x,y),r,r);p->drawLine(QPointF(x,y),QPointF(x,y-r*.6));p->drawLine(QPointF(x,y),QPointF(x+r*.5,y+r*.2));
                } else {p->drawRoundedRect(QRectF(x-r,y-r,2*r,2*r),3,3);for(int i=0;i<5;++i)p->drawLine(QPointF(x-r*.7+i*r*.35,y-r*.5),QPointF(x-r*.7+i*r*.35,y+r*(i%2?.2:.6)));}
            }
            ovHudText(p,QRectF(inset,4,ww-inset-12,hh*.28),ovHudLabel(metric),accent,fs*.48,Qt::AlignLeft|Qt::AlignVCenter);
            ovHudText(p,QRectF(inset,hh*.32,ww-inset-12,hh*.59),ovHudValue(metric,v),fg,fs,Qt::AlignLeft|Qt::AlignVCenter);
        }
        p->restore();
    }
    p->restore();
}
