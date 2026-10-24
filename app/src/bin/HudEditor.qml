import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Column {
    id: root
    property var source
    property var renderer
    property bool snapping: true
    property var layout: ({version:1,widgets:[]})
    property int selected: -1
    property var current: selected>=0 && selected<layout.widgets.length ? layout.widgets[selected] : null
    property var undoStack: []
    property var redoStack: []
    property string gestureBefore: ""
    property string lastSent: ""
    property string error: ""
    property var savedTemplates: []
    property var templateOptions: []
    property string activeTemplateId: ""
    property var templateBaseline: null
    property bool canResetTemplate: templateBaseline!==null
        && JSON.stringify(layout)!==JSON.stringify(templateBaseline)
    property bool hasVariantChoice: current!==null && variants(current.metric).length>1
    property var builtInTemplates: [
        {id:"builtin:campus",name:"校园村落",key:"campus",builtIn:true},
        {id:"builtin:park",name:"公园野外",key:"park",builtIn:true},
        {id:"builtin:simple",name:"简单文字",key:"simple",builtIn:true}
    ]
    property var metrics: ["speed","heading","elevation","heart_rate","elapsed","segment","pace","distance","cadence"]
    property var metricNames: ["速度","航向","海拔","心率","总用时","分段用时","配速","距离","步频"]
    spacing: 7
    function copy(v) { return JSON.parse(JSON.stringify(v)) }
    function reload() {
        if(!source) return
        var raw=String(source.hud_layout===undefined || source.hud_layout===null ? "" : source.hud_layout)
        // 启动顺序：HudEditor 的 Component.onCompleted 先于 SettingsProxy 的（实测如此），
        // 此时 hud_layout 还是空串——不是错误，静默等待 settings 加载完成后的 sync()。
        if(!raw.length) { layout={version:1,widgets:[]}; selected=-1; templateBaseline=null; error=""; return }
        try {
            layout=JSON.parse(raw)
            lastSent=raw; selected=-1;activeTemplateId="";templateBaseline=null;templateChoice.currentIndex=-1;undoStack=[];redoStack=[];error=""
            // Identify an unmodified built-in layout after startup or project loading.
            for(var i=0;i<builtInTemplates.length;i++) {
                var preset=JSON.parse(String(source.hud_theme(builtInTemplates[i].key)))
                if(JSON.stringify(layout)===JSON.stringify(preset)) {
                    activeTemplateId=builtInTemplates[i].id
                    templateBaseline=copy(preset);templateChoice.currentIndex=i
                    break
                }
            }
        } catch(e) {
            layout={version:1,widgets:[]};templateBaseline=null; error="HUD 布局解析失败："+String(e)
        }
    }
    function sync() {
        if(source && source.hud_layout!==lastSent) reload()
    }
    function publish() {
        lastSent=JSON.stringify(layout)
        source.hud_layout=lastSent
        renderer.apply_hud(lastSent)
    }
    function begin() { if(!gestureBefore.length) gestureBefore=JSON.stringify(layout) }
    function end() {
        if(gestureBefore.length && gestureBefore!==JSON.stringify(layout)) {
            undoStack=undoStack.concat([gestureBefore]).slice(-100);redoStack=[]
        }
        gestureBefore=""
    }
    function setField(key,value,continuous) {
        if(!current || (current.locked && key!=="locked" && key!=="visible"))return
        if(!continuous)begin()
        var next=copy(layout);next.widgets[selected][key]=value;layout=next;publish()
        if(!continuous)end()
    }
    function setRect(x,y,w,h) {
        if(!current || current.locked)return
        var n=copy(layout),c=n.widgets[selected]
        c.x=x;c.y=y;c.w=Math.max(.02,Math.min(1,w));c.h=Math.max(.02,Math.min(1,h))
        layout=n;publish()
    }
    function undo() {
        if(!undoStack.length)return
        redoStack=redoStack.concat([JSON.stringify(layout)])
        layout=JSON.parse(undoStack[undoStack.length-1]);undoStack=undoStack.slice(0,-1);selected=-1;publish()
    }
    function redoHistory() {
        if(!redoStack.length)return
        undoStack=undoStack.concat([JSON.stringify(layout)])
        layout=JSON.parse(redoStack[redoStack.length-1]);redoStack=redoStack.slice(0,-1);selected=-1;publish()
    }
    // Restore the whole template, including added/removed widgets and data fields.
    function redo() {
        if(!canResetTemplate)return
        begin()
        layout=copy(templateBaseline);selected=-1;publish();end()
    }
    function variants(metric) {
        if(metric==="speed")return ["digital","arc","dial"]
        if(metric==="heading")return ["digital","compass","ribbon"]
        if(metric==="elevation")return ["digital","profile","scroll"]
        if(metric==="heart_rate")return ["digital","heart"]
        if(metric==="classic")return ["classic"]
        return ["digital"]
    }
    function variantName(v) {
        return ({digital:"数字",arc:"半圆码表",dial:"圆盘码表",compass:"指南针",ribbon:"航向刻度带",profile:"全程剖面",scroll:"滚动曲线",heart:"跳动心形",classic:"经典文字面板"})[v] || v
    }
    function metricName(metric) {
        if(metric==="classic")return "经典面板"
        var index=metrics.indexOf(metric)
        return index>=0 ? metricNames[index] : metric
    }
    function classicIndex() {
        for(var i=0;i<layout.widgets.length;i++)if(layout.widgets[i].metric==="classic")return i
        return -1
    }
    function metricEnabled(metric) {
        var ci=classicIndex()
        if(ci>=0) {
            var fields=layout.widgets[ci].fields
            return Array.isArray(fields) ? fields.indexOf(metric)>=0 : true
        }
        for(var i=0;i<layout.widgets.length;i++)if(layout.widgets[i].metric===metric)return true
        return false
    }
    function defaultWidget(metric,index) {
        var variant=metric==="speed"?"arc":metric==="heading"?"compass":metric==="elevation"?"profile":metric==="heart_rate"?"heart":"digital"
        var wide=metric==="segment" || metric==="elevation"
        return {id:"hud-"+Date.now()+"-"+Math.floor(Math.random()*1000000),metric:metric,variant:variant,
            x:.05+(index%3)*.30,y:.08+Math.floor(index/3)*.22,w:wide ? .34 : .22,h:variant==="digital" ? .12 : .18,
            rotation:0,z:index,opacity:.9,visible:true,locked:false,accent:metric==="heart_rate"?"#ff6577":"#55c9e8",background:"#b3101820",
            foreground:"#ffffff",style:"classic",font_size:32,automatic:true,maximum:30,window:300}
    }
    function setMetricEnabled(metric,enabled) {
        begin()
        var n=copy(layout),ci=classicIndex()
        if(ci>=0) {
            var fields=Array.isArray(n.widgets[ci].fields)?n.widgets[ci].fields.slice():metrics.slice()
            var fi=fields.indexOf(metric)
            if(enabled && fi<0)fields.push(metric)
            if(!enabled && fi>=0)fields.splice(fi,1)
            n.widgets[ci].fields=fields
            selected=ci
        } else if(enabled) {
            var exists=false
            for(var i=0;i<n.widgets.length;i++)if(n.widgets[i].metric===metric)exists=true
            if(!exists && n.widgets.length<100){n.widgets.push(defaultWidget(metric,n.widgets.length));selected=n.widgets.length-1}
        } else {
            for(var j=n.widgets.length-1;j>=0;j--)if(n.widgets[j].metric===metric)n.widgets.splice(j,1)
            selected=-1
        }
        layout=n;publish();end()
    }
    function duplicate() {
        if(!current || layout.widgets.length>=100)return
        begin();var n=copy(layout),c=copy(current);c.id="hud-"+Date.now();c.x+=.02;c.y+=.02;c.locked=false;c.z++
        n.widgets.push(c);layout=n;selected=n.widgets.length-1;publish();end()
    }
    function remove() {
        if(!current || current.locked)return
        begin();var n=copy(layout);n.widgets.splice(selected,1);layout=n;selected=-1;publish();end()
    }
    function refreshTemplates() {
        try { savedTemplates=JSON.parse(source.hud_templates()) }
        catch(e) { savedTemplates=[]; error="HUD 模板读取失败："+String(e) }
        var combined=copy(builtInTemplates)
        savedTemplates.forEach(function(t){combined.push({id:t.id,name:t.name,builtIn:false})})
        templateOptions=combined
        var index=-1
        for(var i=0;i<combined.length;i++)if(combined[i].id===activeTemplateId)index=i
        templateChoice.currentIndex=index
    }
    function applyTemplate(index) {
        if(index<0 || index>=templateOptions.length)return
        var option=templateOptions[index],json=""
        if(option.builtIn)json=source.hud_theme(option.key)
        else {
            var loaded=JSON.parse(source.hud_template_action("load",option.id,""))
            if(!loaded.ok){error=loaded.error;return}
            json=loaded.value
        }
        begin();layout=JSON.parse(json);templateBaseline=copy(layout);selected=-1;publish();end()
        activeTemplateId=option.id;templateChoice.currentIndex=index;error=""
    }
    function saveTemplate(name) {
        var saved=JSON.parse(source.hud_template_action("save","",name))
        if(!saved.ok){error=saved.error;return}
        activeTemplateId=saved.value;templateBaseline=copy(layout);error="";refreshTemplates()
    }
    function deleteTemplate() {
        if(activeTemplateId.indexOf("builtin:")===0 || !activeTemplateId.length)return
        var removed=JSON.parse(source.hud_template_action("delete",activeTemplateId,""))
        if(!removed.ok){error=removed.error;return}
        activeTemplateId="";templateBaseline=null;error="";refreshTemplates()
    }
    Component.onCompleted: {reload();refreshTemplates()}

    Row {
        spacing:4
        Button {text:"撤销";enabled:root.undoStack.length>0;onClicked:root.undo()}
        Button {
            objectName:"resetTemplateButton"
            text:"重做";enabled:root.canResetTemplate;onClicked:root.redo()
            ToolTip.visible:hovered;ToolTip.text:"将所有控件和数据项恢复到所选模板的配置";ToolTip.delay:400
        }
        CheckBox {text:"吸附";checked:true;onToggled:root.snapping=checked}
    }
    RowLayout {
        width:parent.width;spacing:5
        Label {text:"模板"}
        ComboBox {
            id:templateChoice;Layout.fillWidth:true;textRole:"name";model:root.templateOptions;currentIndex:-1
            displayText:currentIndex>=0 && currentIndex<root.templateOptions.length ? root.templateOptions[currentIndex].name : "选择模板"
            onActivated:root.applyTemplate(currentIndex)
        }
        Button {text:"保存模板";onClicked:saveDialog.open()}
        Button {
            text:"删除";visible:root.activeTemplateId.length>0 && root.activeTemplateId.indexOf("builtin:")!==0
            onClicked:deleteDialog.open()
        }
    }
    Label {text:"数据类型"}
    GridLayout {
        width:parent.width;columns:4;columnSpacing:8;rowSpacing:2
        Repeater {
            model:root.metrics
            delegate:CheckBox {
                required property var modelData
                text:root.metricName(modelData)
                checked:root.metricEnabled(modelData)
                onClicked:root.setMetricEnabled(modelData,checked)
            }
        }
    }
    Label {text:"控件列表";font.bold:true}
    Repeater {
        model:root.layout.widgets
        delegate:Button {
            required property int index
            required property var modelData
            width:root.width
            text:(root.selected===index?"● ":"")+root.metricName(modelData.metric)+" · "+root.variantName(modelData.variant)
                +(modelData.locked?" 🔒":"")+(modelData.visible?"":"（隐藏）")
            onClicked:root.selected=index
        }
    }
    Column {
        width:parent.width;spacing:5;visible:root.current!==null
        Label {text:"控件配置"}
        Row {
            spacing:3
            Button {text:"删除";enabled:root.current && !root.current.locked;onClicked:root.remove()}
            CheckBox {text:"显示";checked:root.current?root.current.visible:false;onToggled:root.setField("visible",checked,false)}
            CheckBox {text:"锁定";checked:root.current?root.current.locked:false;onToggled:root.setField("locked",checked,false)}
            Button {text:"前移一层";onClicked:if(root.current)root.setField("z",root.current.z+1,false)}
            Button {text:"后移一层";onClicked:if(root.current)root.setField("z",root.current.z-1,false)}
        }
        RowLayout {
            objectName:"variantChoiceRow"
            visible:root.hasVariantChoice
            width:parent.width
            Label {text:"样式";Layout.preferredWidth:55}
            ComboBox {
                Layout.fillWidth:true;enabled:root.current && !root.current.locked
                property var choices:root.current?root.variants(root.current.metric):[]
                model:choices.map(function(x){return root.variantName(x)})
                currentIndex:root.current?choices.indexOf(root.current.variant):0
                onActivated:root.setField("variant",choices[currentIndex],false)
            }
        }
        GridLayout {
            width:parent.width;columns:2;columnSpacing:8;rowSpacing:4
            Repeater {
                model:[["x","X (%)",100,-100,100],["y","Y (%)",100,-100,100],["w","宽 (%)",100,2,100],
                    ["h","高 (%)",100,2,100],["rotation","旋转 (°)",1,-360,360],["opacity","不透明 (%)",100,0,100],
                    ["font_size","字号",1,8,100]]
                delegate:RowLayout {
                    required property var modelData
                    Layout.fillWidth:true
                    Label {text:modelData[1];Layout.preferredWidth:62;elide:Text.ElideRight}
                    TextField {
                        Layout.fillWidth:true
                        enabled:root.current && !root.current.locked
                        text:root.current?Number(root.current[modelData[0]]*modelData[2]).toFixed(2):""
                        onEditingFinished:{
                            var n=Number(text)
                            if(isFinite(n))root.setField(modelData[0],Math.max(modelData[3],Math.min(modelData[4],n))/modelData[2],false)
                        }
                    }
                }
            }
        }
        GridLayout {
            width:parent.width;columns:2;columnSpacing:8;rowSpacing:4
            Repeater {
                model:[["accent","强调色"],["foreground","文字色"],["background","背景色"]]
                delegate:RowLayout {
                    required property var modelData
                    Layout.fillWidth:true
                    Label {text:modelData[1];Layout.preferredWidth:62}
                    Button {
                        Layout.fillWidth:true
                        enabled:root.current && !root.current.locked
                        text:root.current?root.current[modelData[0]]:""
                        onClicked:{picker.field=modelData[0];picker.selectedColor=root.current[modelData[0]];picker.open()}
                    }
                }
            }
        }
        CheckBox {
            visible:root.current && root.current.metric==="speed"
            text:"自动量程";checked:root.current?root.current.automatic:true
            onToggled:root.setField("automatic",checked,false)
        }
        TextField {
            width:parent.width;visible:root.current && root.current.metric==="speed" && !root.current.automatic
            placeholderText:"最大速度 km/h";text:root.current?String(root.current.maximum):"30"
            onEditingFinished:if(Number(text)>0)root.setField("maximum",Number(text),false)
        }
        TextField {
            width:parent.width;visible:root.current && root.current.variant==="scroll"
            placeholderText:"滚动窗口（秒）";text:root.current?String(root.current.window):"300"
            onEditingFinished:if(Number(text)>=1)root.setField("window",Number(text),false)
        }
    }
    Label {text:root.error;color:"#b00020";wrapMode:Text.Wrap;width:parent.width;visible:text.length>0}
    Label {text:"在画面拖动控件；右下角缩放，上方圆点旋转。方向键微调，Shift 加速。";wrapMode:Text.Wrap;width:parent.width}
    Dialog {
        id:deleteDialog;title:"删除模板？";modal:true
        parent:Overlay.overlay;anchors.centerIn:parent
        standardButtons:Dialog.Ok|Dialog.Cancel
        onAccepted:root.deleteTemplate()
    }
    Dialog {
        id:saveDialog;title:"保存 HUD 模板";modal:true
        parent:Overlay.overlay;anchors.centerIn:parent;width:340
        standardButtons:Dialog.Save|Dialog.Cancel
        onOpened:{templateName.text="";templateName.forceActiveFocus()}
        onAccepted:root.saveTemplate(templateName.text)
        contentItem:TextField {id:templateName;placeholderText:"模板名称";selectByMouse:true}
    }
    HudColorDialog {id:picker;property string field;onAccepted:root.setField(field,selectedColor.toString(),false)}
}
