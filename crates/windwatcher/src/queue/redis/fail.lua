local id = ARGV[1]
local k = key(id)
check_type(k,'hash')
if not fenced(k,ARGV[2],ARGV[3]) then return 0 end
member(k,id)
remove_due(k,id,'leased')
move(k,id,'leased','failed')
clear_lease(k)
redis.call('HSET',k,'failed_at',ARGV[3],'last_error_code',ARGV[4],'last_error_message',ARGV[5])
return 1
