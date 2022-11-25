#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rfd::FileDialog;
use slint::{SharedString, Weak};
use std::sync::{Arc, RwLock};

mod utils;

slint::slint! {
    import { App } from "ui/app.slint";
}
fn main() {
    let app = App::new();

    let handle_weak = app.as_weak();
    app.on_pick_file(move |idx| pick_file(idx, &handle_weak));

    let handle_weak = app.as_weak();
    app.on_pick_file_calback(move |idx, file_spec| set_pick_file(&handle_weak, idx, file_spec));

    let handle_weak = app.as_weak();
    app.on_save_file(move || save_file(&handle_weak));

    let handle_weak = app.as_weak();
    app.on_extract_file(move || extract_file(&handle_weak));

    // 取消保存方法
    let handle_weak = app.as_weak();
    app.on_cancel_job(move || cancel_job(&handle_weak));

    app.run();
}

fn set_waitting_from_thread(handle_weak: &Weak<App>, waitting: bool) {
    let handle = handle_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let handle = handle.unwrap();
        handle.set_waitting(waitting);
    });
}

/// 取消操作
fn cancel_job(handle_weak: &Weak<App>) {
    handle_weak.unwrap().set_user_canceled(true);
}

/// 弹出文件选择对话框
fn pick_file(idx: i32, handle_weak: &Weak<App>) {
    let handle = handle_weak.unwrap();
    if handle.get_waitting() {
        return;
    }
    handle.set_waitting(true);

    let handle_clone = handle_weak.clone();
    std::thread::spawn(move || {
        let res = if idx == 0 {
            //经过测试在文件末尾写入数据不影响文件读取的类型
            utils::pick_file(Some((
                "文件",
                &[
                    "bmp", "png", "jpg", "gif", "exe", "pdf", "jar", "rar", "mp4",
                ],
            )))
        } else {
            //附加文件不限制类型
            utils::pick_file(None)
        };
        let _ = slint::invoke_from_event_loop(move || {
            let handle = handle_clone.unwrap();
            handle.set_waitting(false);
            if let Some(file_spec) = res {
                let _ = handle.invoke_pick_file_calback(idx, FileSpec::from(&file_spec));
            }
        });
    });
}

/// 文件选择回调函数
fn set_pick_file(handle_weak: &Weak<App>, idx: i32, file_spec: FileSpec) {
    let handle = handle_weak.unwrap();
    if idx == 0 {
        handle.set_first_file(file_spec.clone());

        //检查是否存在附加文件
        if let Ok(Some(_)) = utils::check_file(&utils::FileSpec::from(&file_spec)) {
            handle.set_has_attachment(true);
        }
    } else {
        handle.set_second_file(file_spec);
    }
}

/// 保存
fn save_file(handle_weak: &Weak<App>) {
    let handle = handle_weak.unwrap();
    if handle.get_first_file().name.len() == 0
        || handle.get_second_file().name.len() == 0
        || handle.get_waitting()
    {
        return;
    }

    let first_file = utils::FileSpec::from(&handle.get_first_file());
    let append_file = utils::FileSpec::from(&handle.get_second_file());
    let handle_clone = handle_weak.clone();

    handle.set_user_canceled(false);

    std::thread::spawn(move || {
        set_waitting_from_thread(&handle_clone, true);
        let res = utils::get_file_name(
            FileDialog::new()
                .set_file_name(&first_file.name)
                .add_filter(&first_file.extension, &[&first_file.extension])
                .save_file(),
        );

        let handle_clone1 = handle_clone.clone();
        let (output_file_name, _) = res.clone().unwrap_or((String::new(), String::new()));

        let _ = slint::invoke_from_event_loop(move || {
            let handle = handle_clone1.unwrap();
            // 选择文件完成后，就要设置非模式状态，以便处理取消操作
            handle.set_waitting(false);
            if output_file_name.len() > 0 {
                handle.set_output_file(SharedString::from(output_file_name));
                handle.set_current_progress(0);
                handle.set_show_progress(true);
            }
        });

        if let Some((_, output_file_path)) = res {
            // 开始保存文件
            let handle_clone2 = handle_clone.clone();
            // 在UI线程访问这个变量
            let is_cancled = Arc::new(RwLock::new(false));
            let ui_is_cancled = is_cancled.clone();
            let mut copy_success = true;

            let copy_res = utils::copy_file(
                &first_file,
                &append_file,
                &output_file_path,
                move |progress| {
                    let handle_copy = handle_clone2.clone();
                    let ui_is_cancled_copy = ui_is_cancled.clone();
                    //通知UI线程当前进度
                    let _ = slint::invoke_from_event_loop(move || {
                        let handle = handle_copy.unwrap();
                        handle.set_current_progress(progress);
                        //是否取消了当前操作
                        if let (true, Ok(mut ui_is_cancled)) =
                            (handle.get_user_canceled(), ui_is_cancled_copy.write())
                        {
                            *ui_is_cancled = true;
                        }
                    });
                },
                is_cancled,
            );

            let msg = if let Err(err) = copy_res {
                copy_success = false;
                format!("{:?}", err)
            } else {
                "文件保存成功！".to_string()
            };

            //文件保存成功, 更新UI
            let handle_clone3 = handle_clone.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let handle = handle_clone3.unwrap();
                handle.set_current_progress(0);
                handle.set_show_progress(false);
                handle.set_user_canceled(false);
                if copy_success {
                    //复制成功，清空文件
                    handle.set_first_file(slint_generatedApp::FileSpec::default());
                    handle.set_second_file(slint_generatedApp::FileSpec::default());
                }
                alert(&handle, &msg, |_| {});
            });
        }
    });
}

/// 提取文件
fn extract_file(handle_weak: &Weak<App>) {
    let handle = handle_weak.unwrap();
    if handle.get_first_file().name.len() == 0 || handle.get_waitting() {
        return;
    }

    let first_file = utils::FileSpec::from(&handle.get_first_file());

    //读取文件信息
    let attachment_res = utils::check_file(&first_file);
    if attachment_res.is_err() {
        alert(&handle, &format!("{:?}", attachment_res.err()), |_| {});
        return;
    }
    let attachment_res = attachment_res.unwrap();
    if attachment_res.is_none() {
        alert(&handle, "没有附件！", |_| {});
        return;
    }
    let (attachment_file_spec, start_offset, end_offset) = attachment_res.unwrap();
    let handle_clone = handle_weak.clone();
    let attachment_info = format!(
        "附件:{} 大小:{} 确定提取文件吗？",
        attachment_file_spec.name, attachment_file_spec.sizemb
    );
    confirm(&handle, &attachment_info, move |confirm| {
        let handle_clone = handle_clone.clone();
        let attachment_file_spec = attachment_file_spec.clone();
        let first_file = first_file.clone();
        if confirm {
            std::thread::spawn(move || {
                set_waitting_from_thread(&handle_clone, true);
                let res = utils::get_file_name(
                    FileDialog::new()
                        .set_file_name(&attachment_file_spec.name)
                        .add_filter(
                            &attachment_file_spec.extension,
                            &[&attachment_file_spec.extension],
                        )
                        .save_file(),
                );

                let handle_clone1 = handle_clone.clone();
                let (output_file_name, _) = res.clone().unwrap_or((String::new(), String::new()));

                let _ = slint::invoke_from_event_loop(move || {
                    let handle = handle_clone1.unwrap();
                    // 选择文件完成后，就要设置非模式状态，以便处理取消操作
                    handle.set_waitting(false);
                    if output_file_name.len() > 0 {
                        handle.set_output_file(SharedString::from(output_file_name));
                        handle.set_current_progress(0);
                        handle.set_show_progress(true);
                    }
                });

                // 提取文件
                if let Some((_, output_file_path)) = res {
                    let handle_clone2 = handle_clone.clone();
                    // 在UI线程访问这个变量
                    let is_cancled = Arc::new(RwLock::new(false));
                    let ui_is_cancled = is_cancled.clone();
                    let mut copy_success = true;

                    let copy_res = utils::extract_file(
                        &first_file.path,
                        &output_file_path,
                        start_offset,
                        end_offset,
                        move |progress| {
                            let handle_copy = handle_clone2.clone();
                            let ui_is_cancled_copy = ui_is_cancled.clone();
                            //通知UI线程当前进度
                            let _ = slint::invoke_from_event_loop(move || {
                                let handle = handle_copy.unwrap();
                                handle.set_current_progress(progress);
                                //是否取消了当前操作
                                if let (true, Ok(mut ui_is_cancled)) =
                                    (handle.get_user_canceled(), ui_is_cancled_copy.write())
                                {
                                    *ui_is_cancled = true;
                                }
                            });
                        },
                        is_cancled,
                    );

                    let msg = if let Err(err) = copy_res {
                        copy_success = false;
                        format!("{:?}", err)
                    } else {
                        "文件提取成功！".to_string()
                    };

                    //文件保存成功, 更新UI
                    let handle_clone3 = handle_clone.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        let handle = handle_clone3.unwrap();
                        handle.set_current_progress(0);
                        handle.set_show_progress(false);
                        handle.set_user_canceled(false);
                        if copy_success {
                            //复制成功，清空文件
                            handle.set_first_file(slint_generatedApp::FileSpec::default());
                            handle.set_second_file(slint_generatedApp::FileSpec::default());
                        }
                        alert(&handle, &msg, |_| {});
                    });
                }
            });
        }
    });
}

fn confirm<F: Fn(bool) + 'static>(app: &App, msg: &str, callback: F) {
    app.invoke_confirm(SharedString::from(msg));
    app.on_dialog_confirm(callback);
}

fn alert<F: Fn(bool) + 'static>(app: &App, msg: &str, callback: F) {
    app.invoke_alert(SharedString::from(msg));
    app.on_dialog_confirm(callback);
}
