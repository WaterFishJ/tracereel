import QtQuick
import QtQuick.Controls
import QtTest
import "../src/bin" as Hud

Item {
    width:1600;height:720
    QtObject {
        id:source
        property string hud_layout: '{"version":1,"widgets":[]}'
        function hud_templates(){return '[{"id":"123.json","name":"我的模板"}]'}
        function hud_theme(name){
            if(name==="simple")return '{"version":1,"widgets":[{"id":"classic","metric":"classic","variant":"classic","fields":["speed","elapsed"],"x":0.1,"y":0.1,"w":0.2,"h":0.3,"rotation":0,"z":0,"opacity":1,"visible":true,"locked":false,"accent":"#00e5a0","background":"#17202b","foreground":"#ffffff","font_size":18,"automatic":true,"maximum":30,"window":300}]}'
            return '{"version":1,"widgets":[{"id":"built-in","metric":"elapsed","variant":"digital","x":0.1,"y":0.1,"w":0.2,"h":0.1,"rotation":0,"z":0,"opacity":1,"visible":true,"locked":false,"accent":"#00e5a0","background":"#17202b","foreground":"#ffffff","font_size":28,"automatic":true,"maximum":30,"window":300}]}'
        }
        function hud_template_action(action,id,name){
            if(action==="load")return '{"ok":true,"value":'+JSON.stringify(hud_theme("custom"))+'}'
            if(action==="save")return '{"ok":true,"value":"123.json"}'
            return '{"ok":true,"value":""}'
        }
    }
    QtObject {id:renderer;property int updates:0;function apply_hud(json){updates++}}
    Hud.HudEditor {id:editor;source:source;renderer:renderer;x:1280;width:320;visible:false}
    Hud.HudCanvas {id:canvas;editor:editor;width:1280;height:720}
    TestCase {
        name:"HudEditor";when:windowShown
        function init() {editor.visible=false;editor.x=1280;editor.z=0;source.hud_layout='{"version":1,"widgets":[]}';editor.reload();editor.snapping=false}
        function test_add_drag_undo_lock_duplicate() {
            editor.setMetricEnabled("speed",true)
            compare(editor.layout.widgets.length,1)
            editor.begin()
            editor.setRect(.1,.2,.3,.4)
            editor.setRect(.2,.3,.3,.4)
            editor.end()
            compare(editor.undoStack.length,2)
            editor.undo();compare(editor.layout.widgets[0].x,.05)
            editor.redoHistory();compare(editor.layout.widgets[0].x,.2)
            editor.selected=0
            editor.setField("locked",true,false)
            editor.setField("x",.8,false)
            compare(editor.current.x,.2)
            editor.duplicate();compare(editor.layout.widgets.length,2)
            verify(editor.layout.widgets[0].id!==editor.layout.widgets[1].id)
            editor.remove();compare(editor.layout.widgets.length,1)
        }
        function test_mouse_drag_and_keyboard() {
            editor.setMetricEnabled("speed",true);wait(20)
            var x=editor.current.x,y=editor.current.y
            mouseDrag(canvas,(x+.05)*canvas.width,(y+.05)*canvas.height,80,40,Qt.LeftButton)
            verify(editor.current.x>x)
            verify(editor.current.y>y)
            compare(editor.undoStack.length,2)
            editor.undo();compare(editor.layout.widgets[0].x,x)
            editor.selected=0;canvas.forceActiveFocus()
            keyClick(Qt.Key_Right)
            verify(editor.current.x>x)
        }
        function test_resize_rotate_visibility_and_snap() {
            editor.setMetricEnabled("speed",true);wait(20)
            var w=editor.current.w
            mouseDrag(canvas,(editor.current.x+w)*canvas.width,(editor.current.y+editor.current.h)*canvas.height,50,20,Qt.LeftButton)
            verify(editor.current.w>w)
            editor.snapping=true
            compare(canvas.snap(.004,.2,"x"),0)
            editor.setField("visible",false,false);verify(!editor.current.visible)
            editor.setField("visible",true,false)
            editor.setField("rotation",45,false);compare(editor.current.rotation,45)
            editor.setField("z",8,false);compare(editor.current.z,8)
        }
        function test_reload_identifies_campus_without_changing_layout() {
            source.hud_layout=source.hud_theme("campus")
            var updates=renderer.updates
            editor.reload()
            compare(editor.activeTemplateId,"builtin:campus")
            compare(JSON.stringify(editor.layout),JSON.stringify(JSON.parse(source.hud_layout)))
            compare(renderer.updates,updates)
            verify(!editor.canResetTemplate)
            editor.selected=0;editor.setField("x",.6,false)
            editor.reload()
            compare(editor.activeTemplateId,"")
            compare(editor.layout.widgets[0].x,.6)
        }
        function test_combined_template_picker() {
            editor.refreshTemplates()
            compare(editor.templateOptions.length,4)
            editor.applyTemplate(0)
            compare(editor.layout.widgets[0].metric,"elapsed")
            compare(editor.activeTemplateId,"builtin:campus")
            editor.applyTemplate(3)
            compare(editor.activeTemplateId,"123.json")
            editor.selected=0;editor.setField("x",.4,false)
            compare(editor.activeTemplateId,"123.json")
            editor.saveTemplate("我的模板")
            compare(editor.activeTemplateId,"123.json")
            editor.deleteTemplate()
            compare(editor.activeTemplateId,"")
            editor.applyTemplate(2)
            compare(editor.layout.widgets[0].metric,"classic")
            verify(editor.metricEnabled("speed"))
            editor.setMetricEnabled("speed",false)
            verify(!editor.metricEnabled("speed"))
            verify(editor.metricEnabled("elapsed"))
        }
        function test_instruments_resize_width_without_changing_height() {
            var choices=[["heading","compass"],["speed","arc"],["speed","dial"]]
            choices.forEach(function(choice) {
                source.hud_layout='{"version":1,"widgets":[]}';editor.reload()
                editor.setMetricEnabled(choice[0],true)
                editor.setField("variant",choice[1],false)
                wait(20)
                var before=editor.copy(editor.current)
                mouseDrag(canvas,(before.x+before.w)*canvas.width,(before.y+before.h)*canvas.height,60,0,Qt.LeftButton)
                verify(editor.current.w>before.w)
                verify(Math.abs(editor.current.h-before.h)<.001)
                editor.undo()
                compare(editor.current,null)
                compare(editor.layout.widgets[0].w,before.w)
            })
        }
        function test_redo_restores_whole_template_without_selection_and_can_be_undone() {
            editor.applyTemplate(0)
            var original=editor.copy(editor.layout)
            verify(!editor.canResetTemplate)
            editor.selected=0;editor.setField("x",.5,false)
            editor.setField("font_size",44,false)
            editor.setField("accent","#ff0000",false)
            editor.setField("locked",true,false)
            editor.setMetricEnabled("heart_rate",true)
            editor.setField("x",.6,false)
            editor.setMetricEnabled("elapsed",false)
            compare(editor.selected,-1)
            var modified=editor.copy(editor.layout)
            verify(editor.canResetTemplate)
            editor.redo()
            compare(JSON.stringify(editor.layout),JSON.stringify(original))
            compare(editor.selected,-1)
            verify(!editor.canResetTemplate)
            editor.undo()
            compare(JSON.stringify(editor.layout),JSON.stringify(modified))
            verify(editor.canResetTemplate)
            editor.redo()
            compare(JSON.stringify(editor.layout),JSON.stringify(original))
        }
        function test_first_drag_after_template_enables_reset_button() {
            editor.applyTemplate(0);wait(20)
            var original=editor.copy(editor.layout.widgets[0])
            var button=findChild(editor,"resetTemplateButton")
            verify(button!==null)
            verify(!button.enabled)
            mouseDrag(canvas,(original.x+original.w/2)*canvas.width,(original.y+original.h/2)*canvas.height,80,40,Qt.LeftButton)
            verify(editor.current.x>original.x,"First drag must move the newly selected widget")
            verify(button.enabled,"Reset button must enable after dragging")
            editor.x=0;editor.z=10;editor.visible=true;wait(20)
            mouseClick(button,button.width/2,button.height/2,Qt.LeftButton)
            compare(JSON.stringify(editor.layout.widgets[0]),JSON.stringify(original))
            verify(!button.enabled)
        }
        function test_custom_template_baseline_restores_all_widgets() {
            editor.applyTemplate(0);editor.selected=0;editor.setField("x",.3,false)
            editor.setMetricEnabled("heart_rate",true)
            editor.saveTemplate("新模板")
            var baseline=editor.copy(editor.layout)
            editor.setField("font_size",60,false)
            editor.selected=0;editor.setField("x",.8,false)
            editor.setMetricEnabled("speed",true)
            editor.selected=-1
            editor.redo()
            compare(JSON.stringify(editor.layout),JSON.stringify(baseline))
            compare(editor.layout.widgets[0].x,.3)
            verify(!editor.canResetTemplate)
        }
        function test_simple_template_reset_restores_data_checkboxes() {
            editor.applyTemplate(2)
            var baseline=editor.copy(editor.layout)
            editor.setMetricEnabled("speed",false)
            verify(!editor.metricEnabled("speed"))
            editor.setField("x",.5,false)
            editor.selected=-1
            editor.redo()
            compare(JSON.stringify(editor.layout),JSON.stringify(baseline))
            verify(editor.metricEnabled("speed"))
        }
        function test_cadence_digital_and_classic_persist_reload_and_undo() {
            verify(editor.metrics.indexOf("cadence")>=0)
            compare(editor.metricName("cadence"),"步频")
            editor.setMetricEnabled("cadence",true)
            compare(editor.current.metric,"cadence")
            compare(editor.current.variant,"digital")
            verify(!editor.hasVariantChoice)
            var saved=source.hud_layout
            editor.reload()
            verify(editor.metricEnabled("cadence"))
            compare(source.hud_layout,saved)
            editor.setMetricEnabled("cadence",false)
            verify(!editor.metricEnabled("cadence"))
            editor.undo()
            verify(editor.metricEnabled("cadence"))
            editor.applyTemplate(2)
            editor.setMetricEnabled("cadence",true)
            verify(editor.current.fields.indexOf("cadence")>=0)
            editor.reload()
            verify(editor.metricEnabled("cadence"))
            editor.setMetricEnabled("cadence",false)
            verify(!editor.metricEnabled("cadence"))
        }
        function test_style_picker_only_when_multiple_variants() {
            var row=findChild(editor,"variantChoiceRow")
            verify(row!==null);
            ["elapsed","segment","pace","distance","cadence","classic"].forEach(function(metric) {
                source.hud_layout='{"version":1,"widgets":[]}';editor.reload()
                if(metric==="classic"){editor.applyTemplate(2);editor.selected=0}
                else editor.setMetricEnabled(metric,true)
                verify(!editor.hasVariantChoice)
            });
            ["heading","heart_rate","speed","elevation"].forEach(function(metric) {
                source.hud_layout='{"version":1,"widgets":[]}';editor.reload();editor.setMetricEnabled(metric,true)
                verify(editor.hasVariantChoice)
            })
        }
    }
}
