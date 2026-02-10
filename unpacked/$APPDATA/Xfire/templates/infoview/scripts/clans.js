/////////////////////////////////////////////////////////////////////
// clans.js
// - Javascript functions related to the clan templates.
/////////////////////////////////////////////////////////////////////
	
var render_all_clan_news_box = function()
{
    var clan_news_box = document.getElementById('clan_news_box');
    if (!clan_news_box)
        return;

    var all_clan_news = [ %all_clan_news% ];

    if (all_clan_news.length)
    {
        clan_news_box.style.display = "block";

        var strNews = "";
        for (var i = 0; i < all_clan_news.length; ++i)
        {
            strNews += "<div class='indented_boxes'>";
            strNews += "  <div class='box'>";
            strNews += "    <div class='box_title'>";
            strNews += "      <table>";
            strNews += "        <tr>";
            strNews += "          <td rowspan='2'><img src='" + all_clan_news[i].clan_logo + "' alt='" + all_clan_news[i].clan_name + "'/></td>";
            strNews += "          <td>" + all_clan_news[i].clan_name + "</td>";
            strNews += "        </tr>";
            strNews += "        <tr>";
            strNews += "          <td><a href='" + all_clan_news[i].clan_url + "' class='small_text' target='_blank'>" + all_clan_news[i].clan_type_text + "</a></td>";
            strNews += "        </tr>";
            strNews += "      </table>";
            strNews += "    </div>";
            strNews += "    <div class='box_details'>";
            strNews += "      <div><a href='" + all_clan_news[i].news_url + "' target='_blank'>" + all_clan_news[i].news_title + "</a></div>";
            strNews += "      <div>" + all_clan_news[i].news_timestamp + " %js:text_by% " + "<a href='%js:profile_url%" + all_clan_news[i].poster_username + "'>" + all_clan_news[i].poster_display_name + "</a></div>";
            strNews += "    </div>";
            strNews += "  </div>";
            strNews += "</div>";
        }  
  
        var clan_news_details = document.getElementById('clan_news_details');
        if (clan_news_details)
            clan_news_details.innerHTML = strNews;
    }
    else
    {
        clan_news_box.style.display = "none";
    }
}

var render_clan_news_box = function()
{
    var news_box = document.getElementById('news_box');
    if (!news_box)
        return;

    var clan_news = [ %clan_news% ];

    if (clan_news.length)
    {
        news_box.style.display = "block";

        var strNews = "";
        
        for (var i = 0; i < clan_news.length; ++i)
        {
            strNews += "<div><a href='" + clan_news[i].url + "' target='_blank'>" + clan_news[i].title + "</a></div>";
            strNews += "<div>" + clan_news[i].timestamp + " %js:text_by% " + "<a href='%js:profile_url%" + clan_news[i].poster_username + "'>" + clan_news[i].poster_display_name + "</a></div>";
        }
        
        var news_details = document.getElementById('news_details');
        if (news_details)
            news_details.innerHTML = strNews;
    }
    else
    {
        news_box.style.display = "none";
    }
}

var render_clan_events_box = function()
{
    var events_box = document.getElementById('events_box');
    if (!events_box)
        return;

    var clan_events = [ %clan_events% ];

    if (clan_events.length)
    {
        events_box.style.display = "block";

        var strData = "<table>";
        for (var i = 0; i < clan_events.length; ++i)
        {
            strData += "<tr>";
            strData += "<td rowspan='3'><img width='32' height='32' src='" + clan_events[i].icon_url + "'></td>";
            strData += "<td><a href='" + clan_events[i].url + "' target='_blank'>" + clan_events[i].title + "</a></td>";
            strData += "</tr>";
            strData += "<tr><td>" + clan_events[i].event_date + "</td></tr>";
            strData += "<tr><td>" + clan_events[i].event_time + "</td></tr>";
        }
        strData += "</table>";
                
        var events_details = document.getElementById('events_details');
        if (events_details)
            events_details.innerHTML = strData;
    }
    else
    {
        events_box.style.display = "none";
    }
}
       
var render_clan_fav_servers_box = function()
{
    var fav_servers_box = document.getElementById('fav_servers_box');
    if (!fav_servers_box)
        return;

    var clan_fav_servers = [ %clan_favorite_servers% ];
    if (clan_fav_servers.length)
    {
        fav_servers_box.style.display = "block";

        var strData = "";
        for (var i = 0; i < clan_fav_servers.length; ++i)
        {
            strData += "<table width='100%' border='0'";
            if (i%2 != 0)
                strData += " class='odd_table'";
            strData += ">";
            
            var strColorizedLabel = colorizeName(clan_fav_servers[i].label, clan_fav_servers[i].servertype);
            
            strData += "<tr>";
            strData += "<td colspan='4'><a href='" + clan_fav_servers[i].server_url + "' target='_blank'>" + strColorizedLabel + "</a></td>";
            strData += "</tr>";
            strData += "<tr>";
            strData += "<td width='18'><img src='" + clan_fav_servers[i].icon_url + "' width='16' height='16' border='0' /></td>";
            strData += "<td valign='middle'>" + clan_fav_servers[i].game_name + "</td>";
            strData += "<td colspan='2' align='right'><a href='" + clan_fav_servers[i].join_server_url + "' target='_blank'>%text_join%</a></td>";
            strData += "</tr>";
            strData += "</table>";
        }

        var fav_servers_details = document.getElementById('fav_servers_details');
        if (fav_servers_details)
            fav_servers_details.innerHTML = strData;
    }
    else
    {
        fav_servers_box.style.display = "none";
    }
}
        
var render_clan_online_box = function()
{
    var online_box = document.getElementById('online_box');
    if (!online_box)
        return;

    var vOnlineMembers = [ %clan_online_members% ];
    if (vOnlineMembers.length == 0)
    {
        online_box.style.display = "none";
        return;
    }
  
    var strOnline = "%text_members_online%: " + vOnlineMembers.length;
    var online_title = document.getElementById('online_title');
    if (online_title)
        online_title.innerHTML = strOnline;
        
    var online_details = document.getElementById('online_details');
    if (online_details)
    {
        var details = "";
        for (var i = 0; i < vOnlineMembers.length; i++)
        {
            details += "<div class='user_text'><a href='%profile_url%" + vOnlineMembers[i].username + "'>" + vOnlineMembers[i].display + "</a></div>";
        }
        online_details.innerHTML = details;
    }
    
};
        
var render_clan_offline_box = function()
{
    var offline_box = document.getElementById('offline_box');
    if (!offline_box)
        return;

    var vOfflineMembers = [ %clan_offline_members% ];
    if (vOfflineMembers.length == 0)
    {
        offline_box.style.display = "none";
        return;
    }

    var strOffline = "%text_members_offline%: " + vOfflineMembers.length;
    var offline_title = document.getElementById('offline_title');
    if (offline_title)
        offline_title.innerHTML = strOffline;
            
    var offline_details = document.getElementById('offline_details');
    if (offline_details)
    {
        var details = "";
        for (var i = 0; i < vOfflineMembers.length; i++)
        {
            details += "<div class='user_text'><a href='%profile_url%" + vOfflineMembers[i].username + "'>" + vOfflineMembers[i].display + "</a></div>";
        }
        offline_details.innerHTML = details;
    }
    
};
